mod core;
mod term;

use crate::core as c;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    store: Mutex<c::Store>,
    scanner: Mutex<c::Scanner>,
    desktop: Mutex<Option<c::DesktopInstall>>,
    pids: Mutex<HashMap<String, Vec<u32>>>,
}

#[derive(Serialize)]
struct ProfileView {
    name: String,
    color: String,
    principal: bool,
    folder: String,
    folder_short: String,
    desktop_running: bool,
    desktop_used: bool,
    account: Option<String>,
}

#[derive(Serialize)]
struct Snapshot {
    os: &'static str,
    version: String,
    desktop_version: Option<String>,
    profiles: Vec<ProfileView>,
    terminals: Vec<term::TermInfo>,
    /// Terminal que se usa de verdad.
    terminal: String,
    /// Nombre de la terminal guardada si ya no está instalada (la UI avisa).
    terminal_missing: Option<String>,
    claude_found: bool,
    /// Cómo se instala Claude Code desde la app (texto del botón).
    claude_install: Option<String>,
    new_window: bool,
    minimize_to_tray: bool,
    autostart: bool,
    palette: Vec<&'static str>,
}

const OS: &str = if cfg!(target_os = "macos") { "mac" } else if cfg!(windows) { "windows" } else { "other" };

fn snapshot_inner(app: &AppHandle, st: &AppState, deep: bool) -> Snapshot {
    let terminals = term::terminals(); // antes de tomar el lock del store
    let desktop = st.desktop.lock().unwrap().clone();
    let mut store = st.store.lock().unwrap();
    let names = c::profile_names(&store);
    let (all, main) = st.scanner.lock().unwrap().scan(desktop.as_ref(), &names);
    *st.pids.lock().unwrap() = all;
    let profiles = names
        .iter()
        .map(|n| {
            let m = c::meta(&mut store, n).clone();
            ProfileView {
                name: n.clone(),
                color: m.color.clone().unwrap_or_else(|| c::PALETTE[0].into()),
                principal: n == c::PRINCIPAL,
                folder: m.code_folder.clone().unwrap_or_default(),
                folder_short: c::shorten(m.code_folder.as_deref()),
                desktop_running: main.get(n).copied().unwrap_or(false),
                desktop_used: if deep { c::desktop_used(n) } else { true },
                account: if deep { c::code_account(n) } else { None },
            }
        })
        .collect();
    let r = term::resolve(store.terminal.as_deref(), &terminals);
    Snapshot {
        os: OS,
        version: app.package_info().version.to_string(),
        desktop_version: desktop.map(|d| d.version),
        profiles,
        terminals,
        terminal: r.id,
        terminal_missing: r.missing,
        claude_found: if deep { term::claude_found() } else { true },
        claude_install: term::claude_install_hint(),
        new_window: store.new_window,
        minimize_to_tray: store.minimize_to_tray,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        palette: c::PALETTE.to_vec(),
    }
}

#[tauri::command]
async fn snapshot(app: AppHandle, st: State<'_, AppState>, deep: bool) -> Result<Snapshot, String> {
    Ok(snapshot_inner(&app, &st, deep))
}

#[tauri::command]
async fn open_download() -> Result<(), String> {
    term::open_url("https://claude.ai/download")
}

#[tauri::command]
async fn install_tool(id: String) -> Result<String, String> {
    let r = term::install(&id);
    if let Err(e) = &r {
        log_error(e);
    }
    r
}

#[tauri::command]
async fn rescan_terminals() -> Result<(), String> {
    term::invalidate();
    Ok(())
}

#[tauri::command]
async fn refresh_desktop(st: State<'_, AppState>) -> Result<(), String> {
    *st.desktop.lock().unwrap() = c::find_desktop();
    Ok(())
}

#[tauri::command]
async fn open_desktop(st: State<'_, AppState>, name: String) -> Result<(), String> {
    let mut d = st.desktop.lock().unwrap().clone();
    if d.is_none() {
        d = c::find_desktop();
        *st.desktop.lock().unwrap() = d.clone();
    }
    let d = d.ok_or("No encuentro Claude Desktop instalado. Bajalo de claude.ai/download.")?;
    c::launch_desktop(&d, &name)
}

fn do_open_code(st: &AppState, name: &str) -> Result<Option<String>, String> {
    let (folder, color, terminal, new_window) = {
        let mut s = st.store.lock().unwrap();
        let m = c::meta(&mut s, name).clone();
        (m.code_folder, m.color, s.terminal.clone(), s.new_window)
    };
    term::launch_code(&term::CodeLaunch {
        name,
        folder: folder.map(PathBuf::from).unwrap_or_else(c::home),
        color: color.unwrap_or_else(|| c::PALETTE[0].into()),
        saved: terminal,
        new_window,
    })
}

#[tauri::command]
async fn open_code(st: State<'_, AppState>, name: String) -> Result<Option<String>, String> {
    do_open_code(&st, &name)
}

#[tauri::command]
async fn stop_desktop(st: State<'_, AppState>, name: String) -> Result<(), String> {
    let pids = st.pids.lock().unwrap().get(&name).cloned().unwrap_or_default();
    c::kill(&pids);
    Ok(())
}

#[tauri::command]
async fn create_profile(app: AppHandle, st: State<'_, AppState>, name: String, color: String, seed: bool) -> Result<(), String> {
    let n = name.trim().to_string();
    if n.is_empty() {
        return Err("Ponele un nombre.".into());
    }
    if !c::valid_name(&n) {
        return Err("Solo letras, números, espacios, guion y guion bajo (hasta 40).".into());
    }
    {
        let mut s = st.store.lock().unwrap();
        if c::profile_names(&s).iter().any(|x| x.eq_ignore_ascii_case(&n)) {
            return Err("Ya hay un perfil con ese nombre.".into());
        }
        std::fs::create_dir_all(c::profile_dir(&n).join("desktop")).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(c::profile_dir(&n).join("code")).map_err(|e| e.to_string())?;
        if seed {
            c::seed(&n)?;
        }
        s.profiles.retain(|x| !x.name.eq_ignore_ascii_case(&n));
        s.profiles.push(c::ProfileMeta { name: n, color: Some(color), code_folder: None });
        c::save(&s)?;
    }
    rebuild_tray(&app);
    Ok(())
}

#[tauri::command]
async fn delete_profile(app: AppHandle, st: State<'_, AppState>, name: String) -> Result<(), String> {
    if name == c::PRINCIPAL {
        return Err("El perfil Principal no se borra.".into());
    }
    let pids = st.pids.lock().unwrap().get(&name).cloned().unwrap_or_default();
    if !pids.is_empty() {
        c::kill(&pids);
        std::thread::sleep(std::time::Duration::from_millis(1200));
    }
    let dir = c::profile_dir(&name);
    let mut last = None;
    for _ in 0..4 {
        match std::fs::remove_dir_all(&dir) {
            Ok(_) => {
                last = None;
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                last = None;
                break;
            }
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(800));
            }
        }
    }
    if let Some(e) = last {
        return Err(format!("No pude borrar todo. ¿Hay un Claude Code abierto con ese perfil? ({})", e));
    }
    c::remove_shortcuts(&name);
    {
        let mut s = st.store.lock().unwrap();
        s.profiles.retain(|x| !x.name.eq_ignore_ascii_case(&name));
        c::save(&s)?;
    }
    rebuild_tray(&app);
    Ok(())
}

#[tauri::command]
async fn set_color(st: State<'_, AppState>, name: String, color: String) -> Result<(), String> {
    let mut s = st.store.lock().unwrap();
    c::meta(&mut s, &name).color = Some(color);
    c::save(&s)
}

#[tauri::command]
async fn pick_folder(app: AppHandle, st: State<'_, AppState>, name: String) -> Result<bool, String> {
    let current = {
        let mut s = st.store.lock().unwrap();
        c::meta(&mut s, &name).code_folder.clone()
    };
    let mut dlg = app.dialog().file().set_title(format!("Carpeta donde arranca Claude Code · {}", name));
    dlg = dlg.set_directory(current.map(PathBuf::from).unwrap_or_else(c::home));
    let picked = dlg.blocking_pick_folder();
    let Some(p) = picked.and_then(|p| p.into_path().ok()) else { return Ok(false) };
    let mut s = st.store.lock().unwrap();
    c::meta(&mut s, &name).code_folder = Some(p.to_string_lossy().to_string());
    c::save(&s)?;
    Ok(true)
}

#[tauri::command]
async fn reset_folder(st: State<'_, AppState>, name: String) -> Result<(), String> {
    let mut s = st.store.lock().unwrap();
    c::meta(&mut s, &name).code_folder = None;
    c::save(&s)
}

#[tauri::command]
async fn open_data(name: String) -> Result<(), String> {
    let d = if name == c::PRINCIPAL { c::config_dir(&name) } else { c::profile_dir(&name) };
    c::open_folder(&d)
}

#[tauri::command]
async fn shortcuts(st: State<'_, AppState>, name: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let icon = st.desktop.lock().unwrap().as_ref().map(|d| d.exe.clone());
        return c::create_shortcuts(&name, &exe, icon.as_deref());
    }
    #[cfg(not(windows))]
    {
        let _ = (st, name);
        Err("Los atajos están disponibles solo en Windows.".into())
    }
}

#[tauri::command]
async fn set_terminal(st: State<'_, AppState>, id: String) -> Result<(), String> {
    term::invalidate();
    let mut s = st.store.lock().unwrap();
    s.terminal = Some(id);
    c::save(&s)
}

#[tauri::command]
async fn set_new_window(st: State<'_, AppState>, on: bool) -> Result<(), String> {
    let mut s = st.store.lock().unwrap();
    s.new_window = on;
    c::save(&s)
}

#[tauri::command]
async fn set_tray(st: State<'_, AppState>, on: bool) -> Result<(), String> {
    let mut s = st.store.lock().unwrap();
    s.minimize_to_tray = on;
    c::save(&s)
}

#[tauri::command]
async fn set_autostart(app: AppHandle, on: bool) -> Result<(), String> {
    let a = app.autolaunch();
    if on { a.enable() } else { a.disable() }.map_err(|e| e.to_string())
}

#[tauri::command]
async fn quit(app: AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

// ---------------------------------------------------------------- ventana + bandeja

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn rebuild_tray(app: &AppHandle) {
    let Some(tray) = app.tray_by_id("main") else { return };
    if let Ok(m) = tray_menu(app) {
        let _ = tray.set_menu(Some(m));
    }
}

fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let st = app.state::<AppState>();
    let names = c::profile_names(&st.store.lock().unwrap());
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(app, "show", "Abrir Claude Perfiles", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    for n in names {
        let sub = Submenu::new(app, &n, true)?;
        sub.append(&MenuItem::with_id(app, format!("d:{}", n), "Claude Desktop", true, None::<&str>)?)?;
        sub.append(&MenuItem::with_id(app, format!("c:{}", n), "Claude Code", true, None::<&str>)?)?;
        menu.append(&sub)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?)?;
    Ok(menu)
}

/// Mensaje para el usuario cuando la acción no vino de la ventana (bandeja o atajo): si la
/// ventana está visible va como aviso dentro de la app; si no, como diálogo del sistema.
/// `then` corre cuando el usuario lo cierra (o enseguida si no hay nada que mostrar).
fn tell(app: &AppHandle, result: Result<Option<String>, String>, then: impl FnOnce() + Send + 'static) {
    let (msg, err) = match result {
        Ok(None) => return then(),
        Ok(Some(m)) => (m, false),
        Err(e) => {
            log_error(&e);
            (e, true)
        }
    };
    let visible = app.get_webview_window("main").and_then(|w| w.is_visible().ok()).unwrap_or(false);
    if visible {
        use tauri::Emitter;
        let _ = app.emit("notice", serde_json::json!({ "message": msg, "error": err }));
        return then();
    }
    let kind = if err { tauri_plugin_dialog::MessageDialogKind::Error } else { tauri_plugin_dialog::MessageDialogKind::Info };
    app.dialog().message(msg).title("Claude Perfiles").kind(kind).show(move |_| then());
}

fn desktop_action(st: &AppState, n: &str) -> Result<Option<String>, String> {
    let d = st.desktop.lock().unwrap().clone().or_else(c::find_desktop);
    d.ok_or_else(|| "No encuentro Claude Desktop instalado. Bajalo de claude.ai/download.".to_string())
        .and_then(|d| c::launch_desktop(&d, n))
        .map(|_| None)
}

/// Atajos del Escritorio (--desktop/--code NOMBRE). `exit_after`: no había otra instancia
/// abierta, así que la app se cierra después de lanzar (y de mostrar el aviso, si hay).
fn handle_cli(app: &AppHandle, args: &[String], exit_after: bool) -> bool {
    let st = app.state::<AppState>();
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        // Solo en desarrollo: probar la instalación desde una instancia lanzada normalmente.
        #[cfg(debug_assertions)]
        if a == "--install" {
            if let Some(id) = it.next() {
                let r = term::install(id).map(Some);
                let h = app.clone();
                tell(app, r, move || {
                    if exit_after {
                        h.exit(0);
                    }
                });
                return true;
            }
        }
        if a == "--desktop" || a == "--code" {
            if let Some(n) = it.next() {
                let r = if a == "--desktop" { desktop_action(&st, n) } else { do_open_code(&st, n) };
                let h = app.clone();
                tell(app, r, move || {
                    if exit_after {
                        h.exit(0);
                    }
                });
                return true;
            }
        }
    }
    false
}

fn log_error(e: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(c::root().join("error.log")) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "{}  {}", t, e);
    }
}

pub fn run() {
    c::scrub_env();
    let _ = std::fs::create_dir_all(c::root());

    let args: Vec<String> = std::env::args().collect();
    let start_hidden = args.iter().any(|a| a == "--tray");
    let cli_action = args.iter().any(|a| a == "--desktop" || a == "--code" || (cfg!(debug_assertions) && a == "--install"));

    let state = AppState {
        store: Mutex::new(c::load()),
        scanner: Mutex::new(c::Scanner::new()),
        desktop: Mutex::new(c::find_desktop()),
        pids: Mutex::new(HashMap::new()),
    };
    {
        let mut s = state.store.lock().unwrap();
        // Primer arranque: se guarda la terminal del sistema. Si ya había una guardada que no
        // está instalada NO se toca: la UI avisa y el usuario elige.
        if s.terminal.as_deref().map_or(true, |t| t.is_empty()) {
            s.terminal = Some(term::system_default(&term::terminals()));
            let _ = c::save(&s);
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if !handle_cli(app, &argv, false) {
                show_main(app);
            }
        }))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--tray"])))
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            snapshot,
            refresh_desktop,
            rescan_terminals,
            install_tool,
            open_download,
            open_desktop,
            open_code,
            stop_desktop,
            create_profile,
            delete_profile,
            set_color,
            pick_folder,
            reset_folder,
            open_data,
            shortcuts,
            set_terminal,
            set_new_window,
            set_tray,
            set_autostart,
            quit
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // Atajo del Escritorio (--desktop/--code) sin otra instancia abierta: lanzar y salir
            // (después del aviso, si hay uno). Sin bandeja ni ventana.
            if cli_action {
                if !handle_cli(&handle, &std::env::args().collect::<Vec<_>>(), true) {
                    handle.exit(0); // `--code` sin nombre: no hay nada que hacer
                }
                return Ok(());
            }

            let menu = tray_menu(&handle)?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().unwrap())
                .tooltip("Claude Perfiles")
                .menu(&menu)
                .show_menu_on_left_click(cfg!(target_os = "macos"))
                .on_menu_event(|app, ev| {
                    let id = ev.id().as_ref().to_string();
                    let st = app.state::<AppState>();
                    let r = match id.as_str() {
                        "show" => {
                            show_main(app);
                            Ok(())
                        }
                        "quit" => {
                            app.exit(0);
                            Ok(())
                        }
                        _ if id.starts_with("d:") => desktop_action(&st, &id[2..]).map(|_| ()),
                        _ if id.starts_with("c:") => {
                            let r = do_open_code(&st, &id[2..]);
                            tell(app, r, || {});
                            Ok(())
                        }
                        _ => Ok(()),
                    };
                    if let Err(e) = r {
                        tell(app, Err(e), || {});
                    }
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        if !cfg!(target_os = "macos") {
                            show_main(tray.app_handle());
                        }
                    }
                })
                .build(app)?;

            if !start_hidden {
                show_main(&handle);
            }
            Ok(())
        })
        .on_window_event(|win, ev| {
            if let WindowEvent::CloseRequested { api, .. } = ev {
                let app = win.app_handle();
                let tray = app.state::<AppState>().store.lock().unwrap().minimize_to_tray;
                if tray {
                    api.prevent_close();
                    let _ = win.hide();
                } else {
                    app.exit(0);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error al iniciar Claude Perfiles");
}
