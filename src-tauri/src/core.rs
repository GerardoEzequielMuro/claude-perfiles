//! Núcleo: rutas, perfiles, lanzar Desktop/Code y detectar qué está abierto.
//! Desktop: una carpeta --user-data-dir por perfil. Code: un CLAUDE_CONFIG_DIR por perfil.
//!
//! Reglas para no espantar antivirus: nada de PowerShell codificado en base64, nada de WMI,
//! nada de COM, nada de scripts temporales con -ExecutionPolicy Bypass. Todo comando que se
//! lanza es legible en texto plano.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const PRINCIPAL: &str = "Principal";
pub const PALETTE: [&str; 8] = ["#E07A55", "#E3A857", "#9DBB7E", "#6FA8C9", "#D98AA0", "#A7A0C9", "#7FB8B0", "#B8A898"];

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const DETACHED: u32 = 0x0000_0008 | 0x0000_0200; // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP

// ---------------------------------------------------------------- persistencia

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct ProfileMeta {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub code_folder: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Store {
    #[serde(default)]
    pub profiles: Vec<ProfileMeta>,
    #[serde(default = "yes")]
    pub minimize_to_tray: bool,
    #[serde(default)]
    pub terminal: Option<String>,
    #[serde(default)]
    pub new_window: bool,
}
fn yes() -> bool {
    true
}
impl Default for Store {
    fn default() -> Self {
        Store { profiles: vec![], minimize_to_tray: true, terminal: None, new_window: false }
    }
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Windows: %LOCALAPPDATA%\ClaudeProfiles (la misma que usaba la versión 1).
/// Mac: ~/Library/Application Support/ClaudeProfiles.
pub fn root() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(home).join("ClaudeProfiles")
}
fn store_path() -> PathBuf {
    root().join("perfiles.json")
}

pub fn load() -> Store {
    fs::read_to_string(store_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Store>(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save(s: &Store) -> Result<(), String> {
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let tmp = store_path().with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(s).unwrap()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, store_path()).map_err(|e| e.to_string())
}

pub fn valid_name(n: &str) -> bool {
    let n = n.trim();
    !n.is_empty()
        && n.chars().count() <= 40
        && n.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
}

pub fn profile_dir(n: &str) -> PathBuf {
    root().join(n)
}
pub fn data_dir(n: &str) -> Option<PathBuf> {
    if n == PRINCIPAL { None } else { Some(profile_dir(n).join("desktop")) }
}
pub fn config_dir(n: &str) -> PathBuf {
    if n == PRINCIPAL { home().join(".claude") } else { profile_dir(n).join("code") }
}

/// Perfiles que existen en disco (por si alguien borró perfiles.json).
pub fn disk_profiles() -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(root())
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().join("desktop").is_dir() || e.path().join("code").is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

/// Lista ordenada: Principal, los del store (si existen) y los huérfanos del disco.
pub fn profile_names(store: &Store) -> Vec<String> {
    let mut list = vec![PRINCIPAL.to_string()];
    for p in &store.profiles {
        if p.name != PRINCIPAL && profile_dir(&p.name).is_dir() && !list.iter().any(|x| x.eq_ignore_ascii_case(&p.name)) {
            list.push(p.name.clone());
        }
    }
    for d in disk_profiles() {
        if !list.iter().any(|x| x.eq_ignore_ascii_case(&d)) {
            list.push(d);
        }
    }
    list
}

/// Metadatos del perfil; si no existen se crean con un color libre.
pub fn meta<'a>(store: &'a mut Store, n: &str) -> &'a mut ProfileMeta {
    if let Some(i) = store.profiles.iter().position(|x| x.name.eq_ignore_ascii_case(n)) {
        return &mut store.profiles[i];
    }
    let color = if n == PRINCIPAL { PALETTE[0].to_string() } else { free_color(store) };
    store.profiles.push(ProfileMeta { name: n.to_string(), color: Some(color), code_folder: None });
    let _ = save(store);
    store.profiles.last_mut().unwrap()
}

pub fn free_color(store: &Store) -> String {
    let used: Vec<String> = store.profiles.iter().filter_map(|p| p.color.clone()).map(|c| c.to_uppercase()).collect();
    PALETTE.iter().skip(1).find(|c| !used.contains(&c.to_string())).unwrap_or(&PALETTE[1]).to_string()
}

pub fn slug(n: &str) -> String {
    let mut s = String::from("claude-");
    let mut dash = false;
    for c in n.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            s.push(c);
            dash = false;
        } else if !dash {
            s.push('-');
            dash = true;
        }
    }
    s.trim_end_matches('-').to_string()
}

// ---------------------------------------------------------------- entorno limpio

/// Variables que no tienen que pasar a los procesos que lanzamos. Si la app la abrió
/// otra sesión de Claude Code, hereda NO_COLOR=1 y marcadores CLAUDE_CODE_* que dejan
/// a Code sin colores y sin transcript.
pub fn scrub_env() {
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy().to_string();
        let u = k.to_uppercase();
        if u == "NO_COLOR" || u == "CLAUDECODE" || u == "CLAUDE_PID" || u == "CLAUDE_CONFIG_DIR" || u == "GIT_TERMINAL_PROMPT" || u.starts_with("CLAUDE_CODE_") {
            std::env::remove_var(&k);
        }
    }
}

fn spawn(mut c: Command) -> Result<(), String> {
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- Claude Desktop

#[derive(Clone, Debug, Default)]
pub struct DesktopInstall {
    pub exe: PathBuf,
    pub version: String,
    /// Windows MSIX: familia + AppId para abrir el perfil principal como lo abre el menú Inicio.
    pub msix: Option<(String, String)>,
    /// Mac: ruta al .app
    pub bundle: Option<PathBuf>,
}

#[cfg(windows)]
pub fn find_desktop() -> Option<DesktopInstall> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(k) = hk.open_subkey(r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages") {
        let mut best: Option<(Vec<u32>, String, PathBuf)> = None;
        for sub in k.enum_keys().flatten() {
            if !sub.to_lowercase().starts_with("claude_") {
                continue;
            }
            let rootf: String = match k.open_subkey(&sub).and_then(|s| s.get_value("PackageRootFolder")) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let rootp = PathBuf::from(rootf);
            if !rootp.join(r"app\Claude.exe").is_file() {
                continue;
            }
            let ver: Vec<u32> = sub.split('_').nth(1).unwrap_or("").split('.').filter_map(|x| x.parse().ok()).collect();
            if ver.is_empty() {
                continue;
            }
            if best.as_ref().map_or(true, |b| ver > b.0) {
                best = Some((ver, sub.clone(), rootp));
            }
        }
        if let Some((ver, name, rootp)) = best {
            let family = match name.rfind("__") {
                Some(i) => format!("{}_{}", name.split('_').next().unwrap_or("Claude"), &name[i + 2..]),
                None => name.clone(),
            };
            let mut app_id = "Claude".to_string();
            if let Ok(man) = fs::read_to_string(rootp.join("AppxManifest.xml")) {
                // <Application Id="Claude" Executable="app\Claude.exe" ...>
                for part in man.split("<Application ").skip(1) {
                    if part.contains(r#"Executable="app\Claude.exe""#) {
                        if let Some(s) = part.split("Id=\"").nth(1) {
                            app_id = s.split('"').next().unwrap_or("Claude").to_string();
                        }
                        break;
                    }
                }
            }
            return Some(DesktopInstall {
                exe: rootp.join(r"app\Claude.exe"),
                version: ver.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("."),
                msix: Some((family, app_id)),
                bundle: None,
            });
        }
    }
    // Instalador viejo (Squirrel)
    let sq = dirs::data_local_dir()?.join(r"AnthropicClaude\claude.exe");
    if sq.is_file() {
        let version = fs::read_dir(sq.parent()?)
            .ok()?
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|n| n.strip_prefix("app-").map(|v| v.to_string()))
            .max()
            .unwrap_or_default();
        return Some(DesktopInstall { exe: sq, version, msix: None, bundle: None });
    }
    None
}

#[cfg(target_os = "macos")]
pub fn find_desktop() -> Option<DesktopInstall> {
    for app in [PathBuf::from("/Applications/Claude.app"), home().join("Applications/Claude.app")] {
        let exe = app.join("Contents/MacOS/Claude");
        if exe.is_file() {
            let version = fs::read_to_string(app.join("Contents/Info.plist"))
                .ok()
                .and_then(|p| {
                    let i = p.find("<key>CFBundleShortVersionString</key>")?;
                    let rest = &p[i..];
                    let a = rest.find("<string>")? + 8;
                    let b = rest[a..].find("</string>")?;
                    Some(rest[a..a + b].to_string())
                })
                .unwrap_or_default();
            return Some(DesktopInstall { exe, version, msix: None, bundle: Some(app) });
        }
    }
    None
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn find_desktop() -> Option<DesktopInstall> {
    None
}

pub fn launch_desktop(d: &DesktopInstall, n: &str) -> Result<(), String> {
    let dir = data_dir(n);
    if let Some(dir) = &dir {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    {
        match (&dir, &d.msix) {
            (None, Some((fam, id))) => {
                // Igual que abrirlo desde el menú Inicio.
                let mut c = Command::new("explorer.exe");
                c.arg(format!("shell:AppsFolder\\{}!{}", fam, id));
                return spawn(c);
            }
            _ => {
                let mut c = Command::new(&d.exe);
                if let Some(dir) = &dir {
                    c.arg(format!("--user-data-dir={}", dir.display()));
                }
                if let Some(p) = d.exe.parent() {
                    c.current_dir(p);
                }
                c.creation_flags(DETACHED);
                return spawn(c);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let bundle = d.bundle.clone().unwrap_or_else(|| PathBuf::from("/Applications/Claude.app"));
        let mut c = Command::new("/usr/bin/open");
        match &dir {
            None => {
                c.arg("-a").arg(&bundle);
            }
            Some(dir) => {
                // -n: otra instancia. Si ya hay una con esa carpeta, Electron enfoca la existente.
                c.arg("-n").arg("-a").arg(&bundle).arg("--args").arg(format!("--user-data-dir={}", dir.display()));
            }
        }
        return spawn(c);
    }
    #[allow(unreachable_code)]
    Err("Plataforma no soportada".into())
}

pub fn desktop_used(n: &str) -> bool {
    match data_dir(n) {
        None => true,
        Some(d) => d.join("Local Storage").is_dir(),
    }
}

// ---------------------------------------------------------------- detección de ventanas abiertas

fn norm(p: &str) -> String {
    let s = p.trim().trim_matches('"').trim_end_matches(['\\', '/']).replace('\\', "/");
    if cfg!(windows) { s.to_lowercase() } else { s }
}

fn user_data_dir_arg(cmd: &[std::ffi::OsString]) -> Option<String> {
    let mut it = cmd.iter().map(|s| s.to_string_lossy().to_string());
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix("--user-data-dir=") {
            return Some(v.to_string());
        }
        if a == "--user-data-dir" {
            return it.next();
        }
    }
    None
}

pub struct Scanner {
    sys: sysinfo::System,
}

impl Scanner {
    pub fn new() -> Self {
        Scanner { sys: sysinfo::System::new() }
    }

    /// perfil -> PIDs (todos los procesos de esa instancia de Desktop, incluidos los hijos).
    /// `main` solo cuenta los procesos principales (sin --type=).
    pub fn scan(&mut self, d: Option<&DesktopInstall>, names: &[String]) -> (HashMap<String, Vec<u32>>, HashMap<String, bool>) {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
        let mut all: HashMap<String, Vec<u32>> = HashMap::new();
        let mut main: HashMap<String, bool> = HashMap::new();
        let Some(d) = d else { return (all, main) };
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet).with_exe(UpdateKind::OnlyIfNotSet),
        );
        let app_dir = norm(&d.bundle.clone().unwrap_or_else(|| d.exe.parent().map(|p| p.to_path_buf()).unwrap_or_default()).to_string_lossy());
        let by_dir: HashMap<String, String> = names
            .iter()
            .filter_map(|n| data_dir(n).map(|dd| (norm(&dd.to_string_lossy()), n.clone())))
            .collect();
        for (pid, p) in self.sys.processes() {
            let exe = match p.exe() {
                Some(e) => norm(&e.to_string_lossy()),
                None => continue,
            };
            if !exe.starts_with(&app_dir) {
                continue;
            }
            let cmd = p.cmd();
            let is_child = cmd.iter().any(|a| a.to_string_lossy().starts_with("--type="));
            let owner = match user_data_dir_arg(cmd) {
                None => PRINCIPAL.to_string(),
                Some(u) => match by_dir.get(&norm(&u)) {
                    Some(n) => n.clone(),
                    None => continue,
                },
            };
            // En Mac los hijos son "Claude Helper (…)": solo el binario principal cuenta como abierto.
            #[cfg(target_os = "macos")]
            let is_child = is_child || !exe.ends_with("/contents/macos/claude") && !exe.ends_with("/Contents/MacOS/Claude");
            all.entry(owner.clone()).or_default().push(pid.as_u32());
            if !is_child {
                main.insert(owner, true);
            }
        }
        (all, main)
    }
}

pub fn kill(pids: &[u32]) {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    for pid in pids {
        if let Some(p) = sys.process(sysinfo::Pid::from_u32(*pid)) {
            p.kill();
        }
    }
}

// ---------------------------------------------------------------- cuenta de Claude Code

pub fn code_account(n: &str) -> Option<String> {
    let json = if n == PRINCIPAL { home().join(".claude.json") } else { config_dir(n).join(".claude.json") };
    let text = fs::read_to_string(json).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("oauthAccount")?.get("emailAddress")?.as_str().map(|s| s.to_string())
}

/// Copia CLAUDE.md, settings.json y skills/ del perfil principal al nuevo.
pub fn seed(n: &str) -> Result<(), String> {
    let src = home().join(".claude");
    let dst = config_dir(n);
    fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
    for f in ["CLAUDE.md", "settings.json"] {
        if src.join(f).is_file() {
            fs::copy(src.join(f), dst.join(f)).map_err(|e| e.to_string())?;
        }
    }
    if src.join("skills").is_dir() {
        copy_dir(&src.join("skills"), &dst.join("skills")).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let t = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &t)?;
        } else {
            fs::copy(e.path(), t)?;
        }
    }
    Ok(())
}

pub fn shorten(p: Option<&str>) -> String {
    let Some(p) = p.filter(|s| !s.is_empty()) else { return "Tu carpeta de usuario".into() };
    let h = home().to_string_lossy().to_string();
    // Sin distinguir mayúsculas y sin cortar caracteres multibyte (rutas con acentos).
    if p.get(..h.len()).map_or(false, |pre| pre.to_lowercase() == h.to_lowercase()) {
        let rest = &p[h.len()..];
        if rest.is_empty() {
            return "Tu carpeta de usuario".into();
        }
        return format!("~{}", rest);
    }
    p.to_string()
}

pub fn open_folder(p: &Path) -> Result<(), String> {
    fs::create_dir_all(p).ok();
    #[cfg(windows)]
    let c = {
        let mut c = Command::new("explorer.exe");
        c.arg(p);
        c
    };
    #[cfg(target_os = "macos")]
    let c = {
        let mut c = Command::new("/usr/bin/open");
        c.arg(p);
        c
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let c = Command::new("true");
    spawn(c)
}

// ---------------------------------------------------------------- atajos en el Escritorio

#[cfg(windows)]
pub fn create_shortcuts(n: &str, exe: &Path, desktop_icon: Option<&Path>) -> Result<(), String> {
    let desk = dirs::desktop_dir().ok_or("No encuentro el Escritorio")?;
    let mk = |file: String, arg: &str, icon: Option<&Path>| -> Result<(), String> {
        let mut l = mslnk::ShellLink::new(exe).map_err(|e| e.to_string())?;
        l.set_arguments(Some(format!("{} \"{}\"", arg, n)));
        l.set_working_dir(exe.parent().map(|p| p.to_string_lossy().to_string()));
        l.set_icon_location(Some(icon.unwrap_or(exe).to_string_lossy().to_string()));
        l.create_lnk(desk.join(file)).map_err(|e| e.to_string())
    };
    mk(format!("Claude - {}.lnk", n), "--desktop", desktop_icon)?;
    mk(format!("Claude Code - {}.lnk", n), "--code", None)?;
    Ok(())
}

#[cfg(windows)]
pub fn remove_shortcuts(n: &str) {
    if let Some(desk) = dirs::desktop_dir() {
        for f in [format!("Claude - {}.lnk", n), format!("Claude Code - {}.lnk", n)] {
            let _ = fs::remove_file(desk.join(f));
        }
    }
}

#[cfg(not(windows))]
pub fn remove_shortcuts(_: &str) {}
