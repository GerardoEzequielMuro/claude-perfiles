//! Terminales: detectar cuáles hay instaladas y abrir Claude Code en la elegida.
//!
//! Reglas:
//! - Nunca se asume que una terminal existe: se busca en sus rutas reales y, si no está, se marca
//!   como no disponible.
//! - Si la terminal guardada ya no existe NO se cambia la preferencia en silencio: el snapshot la
//!   informa como `missing` (la UI muestra un aviso) y el lanzamiento usa la terminal del sistema
//!   (Windows Terminal/PowerShell, o Terminal en Mac) devolviendo un aviso visible.
//! - Para no espantar antivirus: comandos legibles, nada de base64, WMI, COM ni scripts temporales.

use crate::core::{config_dir, home, slug, PRINCIPAL};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Serialize, Clone, Debug)]
pub struct TermInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub available: bool,
    /// Si soporta "pestaña nueva / ventana nueva".
    pub tabs: bool,
    /// Cómo se instala desde la app si falta (texto para el menú), o None si no se puede.
    pub install: Option<String>,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

/// Resultado de cruzar la preferencia guardada con lo que hay instalado.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Resolved {
    /// Terminal que se va a usar.
    pub id: String,
    /// Si la guardada no está disponible: su nombre (para el aviso).
    pub missing: Option<String>,
}

// ---------------------------------------------------------------- detección

fn exists(p: &Path) -> bool {
    // Los alias de ejecución de la Store (wt.exe en WindowsApps) son reparse points:
    // symlink_metadata los ve aunque metadata() falle.
    p.exists() || fs::symlink_metadata(p).is_ok()
}

/// Busca un ejecutable en el PATH (sin dependencias externas).
#[allow(dead_code)]
fn which(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| exists(p))
}

fn first(cands: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    cands.into_iter().find(|p| exists(p))
}

#[cfg(windows)]
fn detect() -> Vec<TermInfo> {
    let local = dirs::data_local_dir().unwrap_or_default();
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    let sysroot = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));

    let wt = first([local.join(r"Microsoft\WindowsApps\wt.exe")]).or_else(|| which("wt.exe"));
    let warp = first([local.join(r"Programs\Warp\warp.exe"), pf.join(r"Warp\warp.exe")]);
    let tabby = first([local.join(r"Programs\Tabby\Tabby.exe"), pf.join(r"Tabby\Tabby.exe")]);
    let ps = first([sysroot.join(r"System32\WindowsPowerShell\v1.0\powershell.exe")]).or_else(|| which("powershell.exe"));

    vec![
        TermInfo { id: "wt", label: "Windows Terminal", hint: "pestaña con el color del perfil", available: wt.is_some(), tabs: true, install: None, path: wt },
        TermInfo { id: "warp", label: "Warp", hint: "pestañas de color", available: warp.is_some(), tabs: true, install: None, path: warp },
        TermInfo { id: "tabby", label: "Tabby", hint: "moderno, con temas", available: tabby.is_some(), tabs: true, install: None, path: tabby },
        TermInfo { id: "powershell", label: "PowerShell", hint: "una ventana suelta", available: ps.is_some(), tabs: false, install: None, path: ps },
    ]
}

#[cfg(target_os = "macos")]
fn mac_app(bundle: &str, bundle_id: &str) -> Option<PathBuf> {
    let dirs_ = [
        PathBuf::from("/Applications"),
        home().join("Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications/Utilities"),
        PathBuf::from("/Applications/Setapp"),
    ];
    if let Some(p) = dirs_.iter().map(|d| d.join(bundle)).find(|p| p.join("Contents/Info.plist").is_file()) {
        return Some(p);
    }
    // Instalada en otro lado (otra carpeta, otro disco): Spotlight por bundle id.
    let out = Command::new("/usr/bin/mdfind")
        .arg(format!("kMDItemCFBundleIdentifier == '{}'", bundle_id))
        .stderr(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(PathBuf::from)
        .find(|p| p.extension().map_or(false, |e| e == "app") && p.join("Contents/Info.plist").is_file())
}

#[cfg(target_os = "macos")]
fn detect() -> Vec<TermInfo> {
    let terminal = mac_app("Terminal.app", "com.apple.Terminal");
    let iterm = mac_app("iTerm.app", "com.googlecode.iterm2");
    let warp = mac_app("Warp.app", "dev.warp.Warp-Stable");
    let tabby = mac_app("Tabby.app", "org.tabby");
    vec![
        // Terminal viene con macOS: se lista siempre, aunque no se encuentre la ruta.
        TermInfo { id: "terminal", label: "Terminal", hint: "la de macOS", available: true, tabs: false, install: None, path: terminal },
        TermInfo { id: "iterm", label: "iTerm2", hint: "pestañas", available: iterm.is_some(), tabs: true, install: None, path: iterm },
        TermInfo { id: "warp", label: "Warp", hint: "pestañas de color", available: warp.is_some(), tabs: true, install: None, path: warp },
        TermInfo { id: "tabby", label: "Tabby", hint: "moderno, con temas", available: tabby.is_some(), tabs: true, install: None, path: tabby },
    ]
}

#[cfg(not(any(windows, target_os = "macos")))]
fn detect() -> Vec<TermInfo> {
    vec![]
}

static CACHE: Mutex<Option<(Instant, Vec<TermInfo>)>> = Mutex::new(None);

/// Terminales conocidas en esta plataforma, con `available` según lo instalado.
/// Se cachea: la UI pide el estado cada 2 s y en Mac la detección puede consultar Spotlight.
/// Se vuelve a detectar al abrir el menú de terminales, al volver a la ventana y antes de
/// cada lanzamiento (`invalidate`), y como red de seguridad cada 5 minutos.
pub fn terminals() -> Vec<TermInfo> {
    let mut c = CACHE.lock().unwrap();
    if let Some((t, v)) = c.as_ref() {
        if t.elapsed() < Duration::from_secs(300) {
            return v.clone();
        }
    }
    let mut v = detect();
    hide_for_tests(&mut v);
    for t in v.iter_mut().filter(|t| !t.available) {
        t.install = plan(t.id).map(|p| p.hint());
    }
    *c = Some((Instant::now(), v.clone()));
    v
}

/// Solo en builds de desarrollo: CP_HIDE_TERMINALS=warp,wt simula que esas terminales no están
/// instaladas, para probar los avisos y el fallback sin desinstalar nada.
fn hide_for_tests(v: &mut [TermInfo]) {
    #[cfg(debug_assertions)]
    if let Ok(h) = std::env::var("CP_HIDE_TERMINALS") {
        let hide: Vec<&str> = h.split(',').map(|s| s.trim()).collect();
        for t in v.iter_mut().filter(|t| hide.contains(&t.id)) {
            t.available = false;
            t.path = None;
        }
    }
    let _ = v;
}

pub fn invalidate() {
    *CACHE.lock().unwrap() = None;
}

/// Terminal del sistema, en orden: Windows Terminal y después PowerShell en Windows; Terminal en Mac.
const SYSTEM_ORDER: &[&str] = if cfg!(target_os = "macos") { &["terminal"] } else { &["wt", "powershell"] };

pub fn system_default(all: &[TermInfo]) -> String {
    SYSTEM_ORDER
        .iter()
        .find(|id| all.iter().any(|t| t.id == **id && t.available))
        .map(|s| s.to_string())
        .or_else(|| all.iter().find(|t| t.available).map(|t| t.id.to_string()))
        .unwrap_or_else(|| SYSTEM_ORDER[0].to_string())
}

fn known_label(id: &str) -> String {
    match id {
        "wt" => "Windows Terminal".into(),
        "warp" => "Warp".into(),
        "tabby" => "Tabby".into(),
        "powershell" => "PowerShell".into(),
        "terminal" => "Terminal".into(),
        "iterm" => "iTerm2".into(),
        "wezterm" => "WezTerm".into(),
        other => other.to_string(),
    }
}

/// Cruza la preferencia guardada con lo instalado. Función pura (testeable).
pub fn resolve(saved: Option<&str>, all: &[TermInfo]) -> Resolved {
    match saved.filter(|s| !s.is_empty()) {
        None => Resolved { id: system_default(all), missing: None },
        Some(id) => match all.iter().find(|t| t.id == id) {
            Some(t) if t.available => Resolved { id: id.to_string(), missing: None },
            Some(t) => Resolved { id: system_default(all), missing: Some(t.label.to_string()) },
            None => Resolved { id: system_default(all), missing: Some(known_label(id)) },
        },
    }
}

pub fn label_of(id: &str, all: &[TermInfo]) -> String {
    all.iter().find(|t| t.id == id).map(|t| t.label.to_string()).unwrap_or_else(|| known_label(id))
}

// ---------------------------------------------------------------- Claude Code instalado

/// ¿Está el comando `claude`? Solo para avisar: nunca bloquea el lanzamiento, porque la
/// terminal puede tener un PATH más completo que el de esta app (en Mac seguro lo tiene).
pub fn claude_found() -> bool {
    refresh_path();
    let h = home();
    #[cfg(windows)]
    let cands = vec![
        h.join(r".local\bin\claude.exe"),
        dirs::data_local_dir().unwrap_or_default().join(r"Microsoft\WinGet\Links\claude.exe"),
        dirs::data_dir().unwrap_or_default().join(r"npm\claude.cmd"),
        h.join(r".claude\local\claude.exe"),
    ];
    #[cfg(not(windows))]
    let cands = vec![
        h.join(".local/bin/claude"),
        h.join(".claude/local/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
        h.join(".npm-global/bin/claude"),
        h.join(".bun/bin/claude"),
    ];
    if cands.iter().any(|p| exists(p)) {
        return true;
    }
    let exe = if cfg!(windows) { ["claude.exe", "claude.cmd"] } else { ["claude", "claude"] };
    if exe.iter().any(|e| which(e).is_some()) {
        return true;
    }
    // nvm (Mac/Linux): ~/.nvm/versions/node/*/bin/claude
    #[cfg(not(windows))]
    if let Ok(rd) = fs::read_dir(h.join(".nvm/versions/node")) {
        if rd.flatten().any(|e| e.path().join("bin/claude").exists()) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------- lanzar

pub struct CodeLaunch<'a> {
    pub name: &'a str,
    pub folder: PathBuf,
    pub color: String,
    /// Preferencia guardada (puede no existir o no estar instalada).
    pub saved: Option<String>,
    pub new_window: bool,
}

/// Abre Claude Code. Ok(Some(aviso)) cuando hay algo que contarle al usuario
/// (por ejemplo, que se usó otra terminal porque la guardada no está).
pub fn launch_code(l: &CodeLaunch) -> Result<Option<String>, String> {
    invalidate(); // por si se instaló o desinstaló una terminal desde la última vez
    refresh_path(); // por si se instaló Claude Code con la app abierta
    let all = terminals();
    let r = resolve(l.saved.as_deref(), &all);
    let info = all.iter().find(|t| t.id == r.id).cloned();

    let cfg = if l.name == PRINCIPAL { None } else { Some(config_dir(l.name)) };
    if let Some(c) = &cfg {
        fs::create_dir_all(c).map_err(|e| format!("No pude crear la carpeta del perfil ({}).", friendly_io(&e)))?;
    }
    let mut notes = vec![];
    let folder = if l.folder.is_dir() {
        l.folder.clone()
    } else {
        if l.folder != home() {
            notes.push(format!("La carpeta de Code de este perfil ya no existe; abrí en tu carpeta de usuario."));
        }
        home()
    };

    let mut label = label_of(&r.id, &all);
    let mut failed: Option<String> = None;
    if let Err(e) = platform_launch(l, &r.id, info.as_ref(), cfg.as_deref(), &folder) {
        // Si la terminal elegida no arranca, se intenta con la consola de siempre del sistema
        // (PowerShell en Windows, Terminal en Mac) y se avisa: nunca un cambio en silencio.
        let base = SYSTEM_ORDER[SYSTEM_ORDER.len() - 1];
        if r.id == base {
            return Err(format!("No pude abrir {}: {}.", label, e));
        }
        let binfo = all.iter().find(|t| t.id == base).cloned();
        platform_launch(l, base, binfo.as_ref(), cfg.as_deref(), &folder)
            .map_err(|e2| format!("No pude abrir {} ({}) ni {} ({}).", label, e, label_of(base, &all), e2))?;
        failed = Some(format!("{} no arrancó ({})", label, e));
        label = label_of(base, &all);
    }

    let head = match (&r.missing, &failed) {
        (Some(m), Some(f)) => Some(format!("{} no está instalada y {}. Abrí Code en {}. Elegí otra terminal abajo a la izquierda.", m, f, label)),
        (Some(m), None) => Some(format!("{} no está instalada. Abrí Code en {}. Elegí otra terminal abajo a la izquierda.", m, label)),
        (None, Some(f)) => Some(format!("{}. Abrí Code en {}.", f, label)),
        (None, None) => None,
    };
    if let Some(h) = head {
        notes.insert(0, h);
    }
    if !claude_found() {
        notes.push("No encuentro Claude Code instalado: si la terminal dice que no conoce «claude», instalalo desde claude.com/claude-code.".into());
    }
    Ok(if notes.is_empty() { None } else { Some(notes.join(" ")) })
}

fn friendly_io(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "no se encontró el programa".into(),
        std::io::ErrorKind::PermissionDenied => "el sistema no dio permiso".into(),
        // 1920: alias de app de la Store que no se puede ejecutar desde este contexto
        _ if e.raw_os_error() == Some(1920) => "Windows no dejó ejecutarla".into(),
        _ => match e.raw_os_error() {
            Some(n) => format!("error del sistema {}", n),
            None => "error inesperado".into(),
        },
    }
}

fn spawn(mut c: Command) -> Result<(), String> {
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c.spawn().map(|_| ()).map_err(|e| friendly_io(&e))
}

// ---- helpers de comillas (testeados)

fn toml_str(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}
fn yaml_str(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}
/// Comillas simples de PowerShell.
#[allow(dead_code)]
fn ps_q(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}
/// Comillas simples de sh/zsh.
#[allow(dead_code)]
fn sh_q(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}

/// Los colores con nombre que aceptan las pestañas de Warp: el más cercano al del perfil.
fn closest_named(hex: &str) -> &'static str {
    let named: [(&str, (i32, i32, i32)); 6] = [
        ("red", (0xE0, 0x6C, 0x5A)),
        ("yellow", (0xE3, 0xA8, 0x57)),
        ("green", (0x9D, 0xBB, 0x7E)),
        ("cyan", (0x7F, 0xB8, 0xB0)),
        ("blue", (0x6F, 0xA8, 0xC9)),
        ("magenta", (0xD9, 0x8A, 0xA0)),
    ];
    let h = hex.trim_start_matches('#');
    let p = |i: usize| i32::from_str_radix(h.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(128);
    let (r, g, b) = (p(0), p(2), p(4));
    named
        .iter()
        .min_by_key(|(_, (nr, ng, nb))| (r - nr).pow(2) + (g - ng).pow(2) + (b - nb).pow(2))
        .map(|x| x.0)
        .unwrap_or("red")
}

/// Warp: un Tab Config por perfil (`<dir>/claude-<perfil>.toml`) abierto con warp://tab_config.
/// Mismo formato en Windows y Mac. https://docs.warp.dev/terminal/windows/tab-configs/
fn warp_toml(l: &CodeLaunch, folder: &Path, command: &str, shell: Option<&str>) -> String {
    let mut t = format!(
        "name = {}\ntitle = {}\ncolor = {}\n\n[[panes]]\nid = \"main\"\ntype = \"terminal\"\ndirectory = {}\ncommands = [{}]\nis_focused = true\n",
        toml_str(&format!("Claude Code · {}", l.name)),
        toml_str(l.name),
        toml_str(closest_named(&l.color)),
        toml_str(&folder.to_string_lossy()),
        toml_str(command),
    );
    if let Some(s) = shell {
        t.push_str(&format!("shell = {}\n", toml_str(s)));
    }
    t
}

#[allow(dead_code)]
fn open_warp_tab(dir: &Path, l: &CodeLaunch, folder: &Path, command: &str, shell: Option<&str>) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| friendly_io(&e))?;
    // Mismo nombre que la v1 (claude-<perfil>), para no dejar entradas duplicadas en el menú + de Warp.
    let s = slug(l.name);
    fs::write(dir.join(format!("{}.toml", s)), warp_toml(l, folder, command, shell)).map_err(|e| friendly_io(&e))?;
    let url = format!("warp://tab_config/{}{}", s, if l.new_window { "?new_window=true" } else { "" });
    #[cfg(windows)]
    let mut c = Command::new("explorer.exe");
    #[cfg(not(windows))]
    let mut c = Command::new("/usr/bin/open");
    c.arg(url);
    spawn(c)
}

/// Comando para Mac. Asignación como prefijo (`VAR='…' claude`) y no `env VAR=… claude`:
/// la instalación "local" de Claude Code define `claude` como alias del shell, y `env` no ve
/// alias. El prefijo anda en zsh, bash y fish (3.1+). El Principal no toca la variable: la
/// terminal arranca con el entorno limpio del sistema.
#[allow(dead_code)]
fn posix_claude(cfg: Option<&Path>) -> String {
    match cfg {
        Some(c) => format!("CLAUDE_CONFIG_DIR={} claude", sh_q(&c.to_string_lossy())),
        None => "claude".to_string(),
    }
}

#[cfg(any(target_os = "macos", test))]
fn osa_q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// AppleScript para Terminal.app. Si no estaba abierta, al activarla crea una ventana vacía:
/// se escribe en esa en vez de abrir una segunda.
#[cfg(any(target_os = "macos", test))]
fn terminal_script(line: &str) -> String {
    format!(
        r#"set wasRunning to application id "com.apple.Terminal" is running
tell application id "com.apple.Terminal"
	activate
	if wasRunning then
		do script {c}
	else
		repeat 50 times
			if (count of windows) > 0 then exit repeat
			delay 0.1
		end repeat
		if (count of windows) > 0 then
			do script {c} in window 1
		else
			do script {c}
		end if
	end if
end tell"#,
        c = osa_q(line)
    )
}

/// AppleScript para iTerm2: crea la ventana o pestaña y le escribe la línea con `write text`
/// (más robusto que `command`, que depende de cómo iTerm parte los argumentos y cierra la
/// sesión al terminar). Si iTerm no estaba abierto, usa la ventana que crea al arrancar.
#[cfg(any(target_os = "macos", test))]
fn iterm_script(line: &str, new_window: bool) -> String {
    format!(
        r#"set wasRunning to application id "com.googlecode.iterm2" is running
tell application id "com.googlecode.iterm2"
	activate
	if not wasRunning then
		repeat 50 times
			if (count of windows) > 0 then exit repeat
			delay 0.1
		end repeat
	end if
	if (count of windows) = 0 or ({nw} and wasRunning) then
		set w to (create window with default profile)
		set s to current session of w
	else if wasRunning then
		tell current window
			set t to (create tab with default profile)
			set s to current session of t
		end tell
	else
		set s to current session of current window
	end if
	tell s to write text {c}
end tell"#,
        nw = if new_window { "true" } else { "false" },
        c = osa_q(line)
    )
}

/// Solo en builds de desarrollo: deja rastro de qué camino de lanzamiento se usó.
#[allow(unused_variables, dead_code)]
fn debug_log(msg: &str) {
    #[cfg(debug_assertions)]
    {
        use std::io::Write;
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("claude-perfiles-debug.log")) {
            let _ = writeln!(f, "{}", msg);
        }
    }
}

// ---------------------------------------------------------------- instalar desde la app
//
// Todo a la vista: el comando corre en una ventana de terminal que el usuario ve (y puede
// cerrar), con gestores de paquetes oficiales (winget, Homebrew) o el instalador oficial de
// Claude Code. Si no hay gestor, se abre la página de descarga. Nada se instala en segundo plano.

enum Plan {
    /// (comando, gestor): corre en una ventana visible.
    Console(String, &'static str),
    /// Página oficial de descarga.
    Page(&'static str),
}

impl Plan {
    fn hint(&self) -> String {
        match self {
            Plan::Console(_, by) => format!("con {}", by),
            Plan::Page(_) => "abre la descarga".into(),
        }
    }
}

#[cfg(windows)]
fn plan(id: &str) -> Option<Plan> {
    let pkg = match id {
        "claude" => "Anthropic.ClaudeCode",
        "wt" => "Microsoft.WindowsTerminal",
        "warp" => "Warp.Warp",
        "tabby" => "Eugeny.Tabby",
        _ => return None,
    };
    let winget = which("winget.exe").is_some() || exists(&dirs::data_local_dir().unwrap_or_default().join(r"Microsoft\WindowsApps\winget.exe"));
    if winget {
        return Some(Plan::Console(
            format!("winget install --id {} --exact --source winget --accept-package-agreements --accept-source-agreements", pkg),
            "winget",
        ));
    }
    Some(match id {
        // Sin winget se abre la guía oficial en vez de correr `irm … | iex`: descargar y ejecutar
        // un script desde una app sin firmar es justo lo que los antivirus marcan.
        "claude" => Plan::Page("https://code.claude.com/docs/en/setup"),
        "wt" => Plan::Page("ms-windows-store://pdp/?ProductId=9N0DX20HK701"),
        "warp" => Plan::Page("https://www.warp.dev/download"),
        _ => Plan::Page("https://tabby.sh"),
    })
}

#[cfg(target_os = "macos")]
fn brew() -> Option<PathBuf> {
    first([PathBuf::from("/opt/homebrew/bin/brew"), PathBuf::from("/usr/local/bin/brew")])
}

#[cfg(target_os = "macos")]
fn plan(id: &str) -> Option<Plan> {
    // Claude Code: siempre el instalador oficial (se actualiza solo; el de Homebrew no).
    if id == "claude" {
        return Some(Plan::Console("curl -fsSL https://claude.ai/install.sh | bash".into(), "el instalador oficial"));
    }
    let (cask, page) = match id {
        "iterm" => ("iterm2", "https://iterm2.com/downloads.html"),
        "warp" => ("warp", "https://www.warp.dev/download"),
        "tabby" => ("tabby", "https://tabby.sh"),
        _ => return None,
    };
    Some(match brew() {
        Some(b) => Plan::Console(format!("{} install --cask {}", sh_q(&b.to_string_lossy()), cask), "Homebrew"),
        None => Plan::Page(page),
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn plan(_id: &str) -> Option<Plan> {
    None
}

/// Texto del botón para instalar Claude Code (o None si no hay forma en esta plataforma).
pub fn claude_install_hint() -> Option<String> {
    plan("claude").map(|p| p.hint())
}

fn tool_label(id: &str) -> String {
    if id == "claude" {
        "Claude Code".into()
    } else {
        known_label(id)
    }
}

/// Instala una terminal o Claude Code. Devuelve el mensaje para mostrar.
pub fn install(id: &str) -> Result<String, String> {
    let label = tool_label(id);
    let p = plan(id).ok_or_else(|| format!("No sé instalar {} en esta computadora.", label))?;
    match p {
        Plan::Page(url) => {
            open_url(url).map_err(|e| format!("No pude abrir la página de {} ({}).", label, e))?;
            Ok(format!("Abrí la página de descarga de {}. Cuando termines de instalarla, aparece acá sola.", label))
        }
        Plan::Console(cmd, _) => {
            run_visible(&label, &cmd).map_err(|e| format!("No pude abrir la ventana de instalación ({}).", e))?;
            Ok(format!("Se abrió una ventana instalando {}. Seguí ahí los pasos; cuando termine, aparece acá sola.", label))
        }
    }
}

#[cfg(windows)]
fn run_visible(label: &str, cmd: &str) -> Result<(), String> {
    let script = format!(
        "$Host.UI.RawUI.WindowTitle={t}; Write-Host {a} -ForegroundColor Cyan; Write-Host ''; {cmd}; Write-Host ''; Write-Host {b} -ForegroundColor Green",
        t = ps_q(&format!("Instalando {}", label)),
        a = ps_q(&format!("Instalando {} · Claude Perfiles", label)),
        b = ps_q("Terminado. Ya podés cerrar esta ventana y volver a Claude Perfiles."),
    );
    let mut c = Command::new(sys_root().join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    c.arg("-NoLogo").arg("-NoProfile").arg("-NoExit").arg("-Command").arg(script);
    // Consola propia y stdin sin redirigir: winget puede pedir confirmación.
    c.current_dir(home()).creation_flags(CREATE_NEW_CONSOLE);
    c.spawn().map(|_| ()).map_err(|e| friendly_io(&e))
}

#[cfg(target_os = "macos")]
fn run_visible(label: &str, cmd: &str) -> Result<(), String> {
    let line = format!(
        "clear; echo {a}; echo; {cmd} && echo && echo {b}",
        a = sh_q(&format!("Instalando {} · Claude Perfiles", label)),
        b = sh_q("Terminado. Ya podés cerrar esta ventana y volver a Claude Perfiles."),
    );
    osascript(&terminal_script(&line), "Terminal")
}

#[cfg(not(any(windows, target_os = "macos")))]
fn run_visible(_label: &str, _cmd: &str) -> Result<(), String> {
    Err("no disponible".into())
}

pub fn open_url(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    return shell_execute(Path::new(url), "").or_else(|_| {
        // mismo problema que con wt.exe: si ShellExecute no anda desde esta app, un cmd hijo sí
        let mut c = Command::new(sys_root().join(r"System32\cmd.exe"));
        c.raw_arg(format!("/d /c start \"\" \"{}\"", url)).creation_flags(CREATE_NO_WINDOW);
        spawn(c)
    });
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("/usr/bin/open");
        c.arg(url);
        return spawn(c);
    }
    #[allow(unreachable_code)]
    Err(format!("abrí {} a mano", url))
}

#[cfg(windows)]
fn sys_root() -> PathBuf {
    std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

/// Windows: lo recién instalado (winget, instalador de Claude Code) agrega su carpeta al PATH
/// del registro, pero esta app sigue con el PATH con el que arrancó y se lo pasaría a las
/// terminales que abre. Se relee del registro antes de cada lanzamiento.
#[cfg(windows)]
fn refresh_path() {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let read = |root, key: &str| -> Option<String> { RegKey::predef(root).open_subkey(key).ok()?.get_value::<String, _>("Path").ok() };
    let machine = read(HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment");
    let user = read(HKEY_CURRENT_USER, "Environment");
    let fresh: Vec<String> = [machine, user].into_iter().flatten().map(|p| expand_env(&p)).collect();
    if fresh.is_empty() {
        return;
    }
    let mut parts: Vec<String> = fresh.join(";").split(';').filter(|s| !s.is_empty()).map(String::from).collect();
    // Se conserva lo que ya tenía el proceso, para no perder nada.
    if let Some(cur) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&cur) {
            let p = p.to_string_lossy().to_string();
            if !p.is_empty() && !parts.iter().any(|x| x.eq_ignore_ascii_case(&p)) {
                parts.push(p);
            }
        }
    }
    std::env::set_var("PATH", parts.join(";"));
}

#[cfg(not(windows))]
fn refresh_path() {}

/// Expande %VARIABLES% (los valores REG_EXPAND_SZ del registro vienen sin expandir).
#[cfg(any(windows, test))]
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                let name = &after[..j];
                match std::env::var(name) {
                    Ok(v) if !name.is_empty() => out.push_str(&v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ---- Windows

#[cfg(windows)]
fn shell_execute(file: &Path, params: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let w = |s: &std::ffi::OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (verb, f, p) = (w("open".as_ref()), w(file.as_os_str()), w(params.as_ref()));
    // NOASYNC: la activación de apps de la Store termina en otro hilo; sin esto, si la app se
    // cierra enseguida (atajo del Escritorio) la terminal nunca llega a abrirse.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    info.lpVerb = verb.as_ptr();
    info.lpFile = f.as_ptr();
    info.lpParameters = p.as_ptr();
    info.nShow = SW_SHOWNORMAL as i32;
    if unsafe { ShellExecuteExW(&mut info) } != 0 {
        return Ok(());
    }
    Err(match std::io::Error::last_os_error().raw_os_error() {
        Some(2) | Some(3) => "no se encontró el programa".into(),
        Some(5) => "el sistema no dio permiso".into(),
        Some(1920) => "Windows no dejó ejecutarla".into(),
        Some(n) => format!("error del sistema {}", n),
        None => "error inesperado".into(),
    })
}

/// Línea de PowerShell que fija (o limpia) CLAUDE_CONFIG_DIR. Legible, sin base64.
#[cfg(windows)]
fn ps_env_line(cfg: Option<&Path>) -> String {
    match cfg {
        Some(c) => format!("$env:CLAUDE_CONFIG_DIR={}", ps_q(&c.to_string_lossy())),
        None => "Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue".to_string(),
    }
}

/// Argumentos crudos para wt.exe. wt usa ';' para separar sus propios comandos: se escapa '\;'.
#[cfg(any(windows, test))]
fn wt_args(name: &str, color: &str, folder: &str, env_line: &str, new_window: bool, profile: bool) -> Vec<String> {
    let esc = |s: &str| s.replace(';', "\\;");
    let mut v = vec![if new_window { "-w new".to_string() } else { "-w 0".to_string() }, "nt".to_string()];
    if profile {
        v.push("-p \"Claude Code\"".into());
    }
    v.push(format!("--title \"{}\" --suppressApplicationTitle", name.replace('"', "")));
    v.push(format!("--tabColor \"{}\"", color));
    v.push(format!("-d \"{}\"", esc(folder)));
    let ps = format!("{}\\; claude", esc(env_line));
    v.push(format!("powershell.exe -NoLogo -NoExit -Command \"{}\"", ps.replace('"', "\\\"")));
    v
}

#[cfg(windows)]
fn platform_launch(l: &CodeLaunch, id: &str, info: Option<&TermInfo>, cfg: Option<&Path>, folder: &Path) -> Result<(), String> {
    let env_line = ps_env_line(cfg);
    let exe = |fallback: &str| info.and_then(|i| i.path.clone()).unwrap_or_else(|| PathBuf::from(fallback));
    match id {
        "warp" => {
            // Se pide PowerShell explícito: el comando usa su sintaxis. Si la shell no existe,
            // Warp usa la por defecto (documentado en Tab Configs).
            let shell = if which("pwsh.exe").is_some() { "pwsh" } else { "powershell" };
            let dir = dirs::config_dir().unwrap_or_default().join(r"warp\Warp\data\tab_configs");
            open_warp_tab(&dir, l, folder, &format!("{}; claude", env_line), Some(shell))
        }
        "tabby" => {
            let pname = format!("Claude · {}", l.name);
            let changed = ensure_tabby_profile(&tabby_config(), l, cfg, folder, &pname, "powershell.exe", &["-NoLogo", "-NoExit", "-Command", "claude"])?;
            if changed && tabby_running() {
                std::thread::sleep(Duration::from_millis(900)); // que Tabby recargue el config
            }
            let mut c = Command::new(exe("Tabby.exe"));
            c.arg("profile").arg(&pname).current_dir(folder);
            spawn(c)
        }
        "wt" => {
            let profile = ensure_wt_fragment();
            let args = wt_args(l.name, &l.color, &folder.to_string_lossy(), &env_line, l.new_window, profile);
            // wt.exe es un alias de app de la Store. Desde esta app (con WebView2 cargado) ni
            // CreateProcess (error 1920) ni ShellExecute (error 5) lo pueden activar de forma
            // confiable; un `cmd /c start` hijo sí, y además no depende de que esta app siga
            // abierta (atajos del Escritorio). Los argumentos van entre comillas, así que cmd no
            // interpreta & | < > ^ de rutas o nombres.
            let mut c = Command::new(sys_root().join(r"System32\cmd.exe"));
            c.raw_arg(format!("/d /c start \"\" wt.exe {}", args.join(" "))).creation_flags(CREATE_NO_WINDOW);
            spawn(c)
        }
        _ => {
            let title = format!("{} · Claude Code", l.name);
            let mut c = Command::new(exe("powershell.exe"));
            c.arg("-NoLogo").arg("-NoExit").arg("-Command").arg(format!("$Host.UI.RawUI.WindowTitle={}; claude", ps_q(&title)));
            match cfg {
                Some(p) => c.env("CLAUDE_CONFIG_DIR", p),
                None => c.env_remove("CLAUDE_CONFIG_DIR"),
            };
            // Consola propia: sin redirigir stdin, o PowerShell lee EOF y se cierra.
            c.current_dir(folder).creation_flags(CREATE_NEW_CONSOLE);
            c.spawn().map(|_| ()).map_err(|e| friendly_io(&e))
        }
    }
}

// ---- macOS

/// Corre AppleScript esperando el resultado, para poder explicar el error de permisos
/// (Automatización) en vez de fallar en silencio.
#[cfg(target_os = "macos")]
fn osascript(script: &str, app: &str) -> Result<(), String> {
    let out = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| friendly_io(&e))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("-1743") || err.to_lowercase().contains("not authorized") || err.contains("no autoriz") {
        return Err(format!(
            "macOS no le dio permiso a Claude Perfiles para controlar {app}. Andá a Ajustes del Sistema › Privacidad y seguridad › Automatización, activá {app} debajo de Claude Perfiles y probá de nuevo."
        ));
    }
    if err.contains("-1728") || err.contains("-600") || err.contains("-10814") {
        return Err(format!("{app} no respondió. Abrila una vez a mano y probá de nuevo."));
    }
    Err(format!("{} no se pudo abrir ({}).", app, err.trim().lines().last().unwrap_or("error desconocido")))
}

#[cfg(target_os = "macos")]
fn platform_launch(l: &CodeLaunch, id: &str, info: Option<&TermInfo>, cfg: Option<&Path>, folder: &Path) -> Result<(), String> {
    let claude = posix_claude(cfg);
    let line = format!("cd {} && {}", sh_q(&folder.to_string_lossy()), claude);
    let shell = std::env::var("SHELL").ok().filter(|s| Path::new(s).exists()).unwrap_or_else(|| "/bin/zsh".into());
    match id {
        "iterm" => osascript(&iterm_script(&line, l.new_window), "iTerm2"),
        "warp" => open_warp_tab(&home().join(".warp/tab_configs"), l, folder, &claude, None),
        "tabby" => {
            let pname = format!("Claude · {}", l.name);
            // CLAUDE_CONFIG_DIR va en el `env:` del perfil de Tabby. `-i` para que cargue el
            // .zshrc/.bashrc (alias y PATH de quien instaló Claude Code ahí).
            let inner = format!("claude; exec {} -l", shell);
            let changed = ensure_tabby_profile(&tabby_config(), l, cfg, folder, &pname, &shell, &["-l", "-i", "-c", &inner])?;
            if changed && tabby_running() {
                std::thread::sleep(Duration::from_millis(900));
            }
            // El binario directo (no `open --args`): si Tabby ya está abierto, `open` no le pasa
            // los argumentos; el binario sí se los reenvía a la instancia abierta.
            let app = info.and_then(|i| i.path.clone()).unwrap_or_else(|| PathBuf::from("/Applications/Tabby.app"));
            let mut c = Command::new(app.join("Contents/MacOS/Tabby"));
            c.arg("profile").arg(&pname).current_dir(folder);
            spawn(c)
        }
        _ => osascript(&terminal_script(&line), "Terminal"),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_launch(_: &CodeLaunch, _: &str, _: Option<&TermInfo>, _: Option<&Path>, _: &Path) -> Result<(), String> {
    Err("esta plataforma no está soportada".into())
}

// ---- Tabby (Windows y Mac)

fn tabby_config() -> PathBuf {
    // Windows: %APPDATA%\tabby\config.yaml · Mac: ~/Library/Application Support/tabby/config.yaml
    dirs::config_dir().unwrap_or_default().join("tabby").join("config.yaml")
}

fn tabby_running() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, sysinfo::ProcessRefreshKind::nothing());
    sys.processes().values().any(|p| {
        let n = p.name().to_string_lossy().to_lowercase();
        n == "tabby.exe" || n == "tabby"
    })
}

/// Mete (o actualiza) el perfil "claude-perfiles-<slug>" en `profiles:` del config.yaml de Tabby
/// sin tocar los demás. Devuelve true si cambió el archivo. Hace un backup la primera vez.
fn ensure_tabby_profile(path: &Path, l: &CodeLaunch, cfg: Option<&Path>, folder: &Path, pname: &str, command: &str, args: &[&str]) -> Result<bool, String> {
    let result = tabby_yaml(&fs::read_to_string(path).unwrap_or_default(), l.name, &l.color, cfg, folder, pname, command, args);
    let Some(result) = result else { return Ok(false) };
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).ok();
    }
    if path.is_file() {
        let bak = path.with_extension("yaml.bak-claudeperfiles");
        if !bak.exists() {
            fs::copy(path, bak).ok();
        }
    }
    fs::write(path, result).map_err(|e| format!("no pude escribir la configuración de Tabby ({})", friendly_io(&e)))?;
    Ok(true)
}

/// Parte pura de lo anterior (testeable). None si no hay cambios.
#[allow(clippy::too_many_arguments)]
fn tabby_yaml(text: &str, name: &str, color: &str, cfg: Option<&Path>, folder: &Path, pname: &str, command: &str, args: &[&str]) -> Option<String> {
    let id = format!("local:custom:claude-perfiles-{}", slug(name));
    let mut item = vec![
        "  - type: local".to_string(),
        format!("    name: {}", yaml_str(pname)),
        "    group: 'Claude Perfiles'".to_string(),
        "    icon: fas fa-terminal".to_string(),
        format!("    color: {}", yaml_str(color)),
        "    disableDynamicTitle: true".to_string(),
        "    options:".to_string(),
        format!("      command: {}", yaml_str(command)),
        "      args:".to_string(),
    ];
    for a in args {
        item.push(format!("        - {}", yaml_str(a)));
    }
    item.push(format!("      cwd: {}", yaml_str(&folder.to_string_lossy())));
    match cfg {
        Some(c) => {
            item.push("      env:".to_string());
            item.push(format!("        CLAUDE_CONFIG_DIR: {}", yaml_str(&c.to_string_lossy())));
        }
        None => item.push("      env: {}".to_string()),
    }
    item.push(format!("    id: {}", id));

    let original = if text.trim().is_empty() { "version: 8\nprofiles: []\n".to_string() } else { text.replace("\r\n", "\n") };
    let mut lines: Vec<String> = original.split('\n').map(|s| s.to_string()).collect();
    let (start, end, mut kept) = match lines.iter().position(|l| l.starts_with("profiles:")) {
        None => {
            lines.insert(0, "profiles:".into());
            (0, 1, vec![])
        }
        Some(s) => {
            let mut e = s + 1;
            while e < lines.len() && (lines[e].starts_with(' ') || lines[e].starts_with('-') || lines[e].is_empty()) {
                e += 1;
            }
            let mut kept = vec![];
            let mut cur: Vec<String> = vec![];
            let flush = |cur: &mut Vec<String>, kept: &mut Vec<String>| {
                if !cur.is_empty() && !cur.iter().any(|x| x.trim() == format!("id: {}", id)) {
                    kept.append(cur);
                }
                cur.clear();
            };
            for l in &lines[s + 1..e] {
                if l.starts_with("  - ") || l.starts_with("- ") {
                    flush(&mut cur, &mut kept);
                }
                if !l.is_empty() {
                    cur.push(l.clone());
                }
            }
            flush(&mut cur, &mut kept);
            (s, e, kept)
        }
    };
    kept.extend(item);
    let mut block = vec!["profiles:".to_string()];
    block.extend(kept);
    lines.splice(start..end, block);
    let result = lines.join("\n");
    if result == original { None } else { Some(result) }
}

// ---- Windows Terminal: perfil «Claude Code» con un tema cálido (fragment, no toca settings.json)

/// Devuelve true si el perfil «Claude Code» está disponible para usar con -p.
#[cfg(windows)]
fn ensure_wt_fragment() -> bool {
    let dir = dirs::data_local_dir().unwrap_or_default().join(r"Microsoft\Windows Terminal\Fragments\ClaudePerfiles");
    let f = dir.join("claude.json");
    if f.is_file() {
        return true;
    }
    // Recién creado: un Windows Terminal ya abierto no lo ve hasta reiniciarse, así que esta vez
    // se abre con el perfil por defecto (si no, wt muestra un error de perfil inexistente).
    let _ = fs::create_dir_all(&dir).and_then(|_| fs::write(&f, WT_FRAGMENT));
    false
}

#[cfg(windows)]
const WT_FRAGMENT: &str = r##"{
  "profiles": [
    {
      "guid": "{6b1d2c3e-7a55-4e07-9c1a-140e0b2e221c}",
      "name": "Claude Code",
      "commandline": "powershell.exe -NoLogo",
      "colorScheme": "Claude Perfiles",
      "font": { "face": "Cascadia Mono", "size": 11 },
      "padding": "18, 12, 18, 10",
      "cursorShape": "bar",
      "cursorColor": "#E07A55",
      "useAcrylic": false,
      "opacity": 97,
      "suppressApplicationTitle": true,
      "hidden": false
    }
  ],
  "schemes": [
    {
      "name": "Claude Perfiles",
      "background": "#140E0B", "foreground": "#F4EDE8", "cursorColor": "#E07A55", "selectionBackground": "#4A2A1C",
      "black": "#2A201B", "red": "#E5735F", "green": "#9DBB7E", "yellow": "#E3A857", "blue": "#6FA8C9", "purple": "#D98AA0", "cyan": "#7FB8B0", "white": "#D8CCC4",
      "brightBlack": "#6E5D54", "brightRed": "#F08E7A", "brightGreen": "#B5D196", "brightYellow": "#F0C27A", "brightBlue": "#8FC0DE", "brightPurple": "#E9A5B8", "brightCyan": "#9DD0C8", "brightWhite": "#F4EDE8"
    }
  ]
}
"##;

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &'static str, available: bool) -> TermInfo {
        TermInfo { id, label: Box::leak(known_label(id).into_boxed_str()), hint: "", available, tabs: true, install: None, path: None }
    }
    fn win(wt: bool, warp: bool, tabby: bool) -> Vec<TermInfo> {
        vec![t("wt", wt), t("warp", warp), t("tabby", tabby), t("powershell", true)]
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn resolve_windows_matrix() {
        // primer arranque: terminal del sistema, sin aviso
        assert_eq!(resolve(None, &win(true, true, true)), Resolved { id: "wt".into(), missing: None });
        assert_eq!(resolve(None, &win(false, true, false)), Resolved { id: "powershell".into(), missing: None });
        // Warp guardado y presente
        assert_eq!(resolve(Some("warp"), &win(true, true, false)), Resolved { id: "warp".into(), missing: None });
        // Warp guardado y desinstalado: aviso + Windows Terminal
        assert_eq!(resolve(Some("warp"), &win(true, false, false)), Resolved { id: "wt".into(), missing: Some("Warp".into()) });
        // Warp guardado, sin Windows Terminal: aviso + PowerShell
        assert_eq!(resolve(Some("warp"), &win(false, false, false)), Resolved { id: "powershell".into(), missing: Some("Warp".into()) });
        // Tabby presente
        assert_eq!(resolve(Some("tabby"), &win(false, false, true)), Resolved { id: "tabby".into(), missing: None });
        // terminal vieja que ya no se ofrece (WezTerm) o de otra plataforma (iTerm2)
        assert_eq!(resolve(Some("wezterm"), &win(true, false, false)).missing, Some("WezTerm".into()));
        assert_eq!(resolve(Some("iterm"), &win(true, false, false)), Resolved { id: "wt".into(), missing: Some("iTerm2".into()) });
        // texto vacío = sin preferencia
        assert_eq!(resolve(Some(""), &win(true, false, false)).missing, None);
    }

    #[test]
    fn resolve_mac_shape() {
        let mac = vec![t("terminal", true), t("iterm", false), t("warp", false), t("tabby", false)];
        // en Mac el orden del sistema empieza por Terminal; en este test (Windows) cae al primero disponible
        let r = resolve(Some("warp"), &mac);
        assert_eq!(r.missing, Some("Warp".into()));
        assert_eq!(r.id, "terminal");
    }

    #[test]
    fn quoting() {
        assert_eq!(ps_q("C:\\Users\\O'Brien\\x"), "'C:\\Users\\O''Brien\\x'");
        assert_eq!(sh_q("/Users/o'brien/Mis Proyectos"), "'/Users/o'\\''brien/Mis Proyectos'");
        assert_eq!(yaml_str("José's"), "'José''s'");
        assert_eq!(toml_str("C:\\a \"b\""), "\"C:\\\\a \\\"b\\\"\"");
    }

    #[test]
    fn wt_args_escape() {
        let a = wt_args("Cliente Ñandú", "#9DBB7E", "C:\\Users\\José Pérez\\a;b", "$env:CLAUDE_CONFIG_DIR='C:\\x;y'", false, true);
        assert_eq!(a[0], "-w 0");
        assert!(a.contains(&"-p \"Claude Code\"".to_string()));
        assert!(a.iter().any(|x| x == "-d \"C:\\Users\\José Pérez\\a\\;b\""), "{:?}", a);
        let last = a.last().unwrap();
        assert!(last.contains("C:\\x\\;y"));
        assert!(last.ends_with("\\; claude\""));
        let b = wt_args("X", "#000000", "C:\\", "e", true, false);
        assert_eq!(b[0], "-w new");
        assert!(!b.iter().any(|x| x.contains("-p ")));
    }

    #[test]
    fn tabby_merge_keeps_other_profiles_and_is_idempotent() {
        let original = "version: 8\nprofiles:\n  - type: ssh\n    name: Mi server\n    id: ssh:custom:abc\ngroups: []\n";
        let cfg = PathBuf::from("C:\\Users\\Jo sé\\code");
        let folder = PathBuf::from("C:\\Users\\Jo sé");
        let once = tabby_yaml(original, "Trabajo", "#6FA8C9", Some(&cfg), &folder, "Claude · Trabajo", "powershell.exe", &["-Command", "claude"]).unwrap();
        assert!(once.contains("name: Mi server"));
        assert!(once.contains("id: local:custom:claude-perfiles-claude-trabajo"));
        assert!(once.contains("CLAUDE_CONFIG_DIR: 'C:\\Users\\Jo sé\\code'"));
        assert!(once.contains("groups: []"));
        // segunda vez: sin cambios
        assert!(tabby_yaml(&once, "Trabajo", "#6FA8C9", Some(&cfg), &folder, "Claude · Trabajo", "powershell.exe", &["-Command", "claude"]).is_none());
        // cambio de color: reemplaza, no duplica
        let recolor = tabby_yaml(&once, "Trabajo", "#E07A55", Some(&cfg), &folder, "Claude · Trabajo", "powershell.exe", &["-Command", "claude"]).unwrap();
        assert_eq!(recolor.matches("claude-perfiles-claude-trabajo").count(), 1);
        assert!(recolor.contains("'#E07A55'"));
        // config vacío o inexistente
        let fresh = tabby_yaml("", "Principal", "#E07A55", None, &folder, "Claude · Principal", "powershell.exe", &[]).unwrap();
        assert!(fresh.starts_with("version: 8\nprofiles:\n  - type: local"));
        assert!(fresh.contains("env: {}"));
    }

    #[test]
    fn mac_commands() {
        let cfg = PathBuf::from("/Users/josé/Library/Application Support/ClaudeProfiles/Cliente Ñ/code");
        assert_eq!(posix_claude(Some(&cfg)), "CLAUDE_CONFIG_DIR='/Users/josé/Library/Application Support/ClaudeProfiles/Cliente Ñ/code' claude");
        assert_eq!(posix_claude(None), "claude");
        let line = format!("cd {} && {}", sh_q("/Users/o'brien/Mis \"Proyectos\""), posix_claude(Some(&cfg)));
        let t = terminal_script(&line);
        // las comillas dobles y las barras quedan escapadas para AppleScript
        assert!(t.contains("\\\"Proyectos\\\""));
        assert!(t.contains("do script \"cd "));
        assert!(t.contains("application id \"com.apple.Terminal\""));
        let i = iterm_script(&line, false);
        assert!(i.contains("write text \"cd "));
        assert!(i.contains("(false and wasRunning)"));
        assert!(iterm_script(&line, true).contains("(true and wasRunning)"));
    }

    /// Solo en macOS (CI): que AppleScript acepte los scripts generados. Necesita iTerm2
    /// instalado para compilar su diccionario; el workflow lo instala con Homebrew.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn mac_applescripts_compile() {
        let line = format!("cd {} && {}", sh_q("/Users/o'brien/Mis \"Proyectos\""), posix_claude(Some(Path::new("/tmp/a b/code"))));
        for (name, script) in [("terminal", terminal_script(&line)), ("iterm-tab", iterm_script(&line, false)), ("iterm-win", iterm_script(&line, true))] {
            let out = std::env::temp_dir().join(format!("cp-{}.scpt", name));
            let r = Command::new("/usr/bin/osacompile").arg("-e").arg(&script).arg("-o").arg(&out).output().unwrap();
            assert!(r.status.success(), "{} no compila: {}\n{}", name, String::from_utf8_lossy(&r.stderr), script);
        }
    }

    /// Solo en macOS (CI): la detección encuentra Terminal y, si el workflow los instaló,
    /// iTerm2/Warp/Tabby en sus rutas reales. CP_EXPECT=iterm,warp,tabby
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn mac_detect_real() {
        let all = detect();
        let term = all.iter().find(|t| t.id == "terminal").unwrap();
        assert!(term.path.is_some(), "no encontré Terminal.app");
        for id in std::env::var("CP_EXPECT").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
            let t = all.iter().find(|t| t.id == id).unwrap();
            assert!(t.available, "{} instalado pero no detectado: {:?}", id, all);
        }
        assert_eq!(resolve(None, &all).id, "terminal");
    }

    /// Solo en Windows: la detección real encuentra PowerShell y el resultado es coherente.
    #[cfg(windows)]
    #[test]
    fn windows_detect_real() {
        let all = detect();
        assert!(all.iter().find(|t| t.id == "powershell").unwrap().available);
        let r = resolve(None, &all);
        assert!(r.id == "wt" || r.id == "powershell");
        for t in &all {
            assert_eq!(t.available, t.path.is_some(), "{:?}", t);
        }
    }

    #[test]
    fn expand_env_vars() {
        std::env::set_var("CP_TEST_X", "C:\\Users\\Jo");
        assert_eq!(expand_env("%CP_TEST_X%\\bin;%NO_EXISTE_CP%;100%"), "C:\\Users\\Jo\\bin;%NO_EXISTE_CP%;100%");
    }

    #[test]
    fn install_plans() {
        // toda terminal que puede faltar tiene forma de instalarse, y Claude Code también
        for t in detect() {
            if t.id != "powershell" && t.id != "terminal" {
                assert!(plan(t.id).is_some(), "{} sin plan de instalación", t.id);
            }
        }
        assert!(plan("claude").is_some());
        assert!(plan("inventada").is_none());
    }

    #[test]
    fn warp_color() {
        assert_eq!(closest_named("#E07A55"), "red");
        assert_eq!(closest_named("#6FA8C9"), "blue");
        // un color inválido no rompe: devuelve algún nombre válido
        assert!(["red", "yellow", "green", "cyan", "blue", "magenta"].contains(&closest_named("garbage")));
    }
}


