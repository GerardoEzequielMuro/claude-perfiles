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
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
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
    let lp = p.to_lowercase();
    if lp.starts_with(&h.to_lowercase()) {
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

// ---------------------------------------------------------------- terminales

#[derive(Serialize, Clone)]
pub struct TermInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub available: bool,
    pub tabs: bool,
}

#[cfg(windows)]
fn local_programs(rel: &str) -> PathBuf {
    dirs::data_local_dir().unwrap_or_default().join("Programs").join(rel)
}
#[cfg(windows)]
fn warp_exe() -> PathBuf {
    local_programs(r"Warp\warp.exe")
}
#[cfg(windows)]
fn tabby_exe() -> PathBuf {
    local_programs(r"Tabby\Tabby.exe")
}
#[cfg(windows)]
fn wt_exe() -> PathBuf {
    dirs::data_local_dir().unwrap_or_default().join(r"Microsoft\WindowsApps\wt.exe")
}
#[cfg(target_os = "macos")]
fn mac_app(name: &str) -> Option<PathBuf> {
    [PathBuf::from("/Applications").join(name), home().join("Applications").join(name)].into_iter().find(|p| p.is_dir())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn terminals() -> Vec<TermInfo> {
    vec![]
}

#[cfg(windows)]
pub fn terminals() -> Vec<TermInfo> {
    vec![
        TermInfo { id: "wt", label: "Windows Terminal", hint: "pestaña con el color del perfil", available: wt_exe().is_file(), tabs: true },
        TermInfo { id: "warp", label: "Warp", hint: "pestañas de color, bloques", available: warp_exe().is_file(), tabs: true },
        TermInfo { id: "tabby", label: "Tabby", hint: "moderno, con temas", available: tabby_exe().is_file(), tabs: true },
        TermInfo { id: "powershell", label: "PowerShell", hint: "una ventana suelta", available: true, tabs: false },
    ]
}

#[cfg(target_os = "macos")]
pub fn terminals() -> Vec<TermInfo> {
    vec![
        TermInfo { id: "terminal", label: "Terminal", hint: "la de macOS", available: true, tabs: false },
        TermInfo { id: "iterm", label: "iTerm2", hint: "pestañas", available: mac_app("iTerm.app").is_some(), tabs: true },
        TermInfo { id: "warp", label: "Warp", hint: "pestañas de color, bloques", available: mac_app("Warp.app").is_some(), tabs: false },
        TermInfo { id: "tabby", label: "Tabby", hint: "moderno, con temas", available: mac_app("Tabby.app").is_some(), tabs: true },
    ]
}

pub fn default_terminal() -> String {
    terminals().into_iter().find(|t| t.available).map(|t| t.id.to_string()).unwrap_or_default()
}

pub fn resolve_terminal(t: Option<&str>) -> String {
    let all = terminals();
    match t {
        Some(id) if all.iter().any(|x| x.id == id && x.available) => id.to_string(),
        _ => default_terminal(),
    }
}

fn toml_str(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}
fn yaml_str(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}
/// Comillas simples de PowerShell.
fn ps_q(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}
/// Comillas simples de sh/zsh.
#[allow(dead_code)]
fn sh_q(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}

#[allow(dead_code)]
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

pub struct CodeLaunch<'a> {
    pub name: &'a str,
    pub folder: PathBuf,
    pub color: String,
    pub terminal: String,
    pub new_window: bool,
}

pub fn launch_code(l: &CodeLaunch) -> Result<(), String> {
    let cfg = if l.name == PRINCIPAL { None } else { Some(config_dir(l.name)) };
    if let Some(c) = &cfg {
        fs::create_dir_all(c).map_err(|e| e.to_string())?;
    }
    let folder = if l.folder.is_dir() { l.folder.clone() } else { home() };
    platform_launch_code(l, cfg.as_deref(), &folder)
}

#[cfg(windows)]
fn platform_launch_code(l: &CodeLaunch, cfg: Option<&Path>, folder: &Path) -> Result<(), String> {
    let title = format!("{} · Claude Code", l.name);
    // Una línea de PowerShell legible, sin base64.
    let env_line = match cfg {
        Some(c) => format!("$env:CLAUDE_CONFIG_DIR={}", ps_q(&c.to_string_lossy())),
        None => "Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue".to_string(),
    };
    match l.terminal.as_str() {
        "warp" => {
            let dir = dirs::config_dir().unwrap_or_default().join(r"warp\Warp\data\tab_configs");
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let s = slug(l.name);
            let toml = format!(
                "name = {}\ntitle = {}\ncolor = {}\n\n[[panes]]\nid = \"main\"\ntype = \"terminal\"\ndirectory = {}\ncommands = [{}]\nis_focused = true\n",
                toml_str(&format!("Claude Code · {}", l.name)),
                toml_str(l.name),
                toml_str(closest_named(&l.color)),
                toml_str(&folder.to_string_lossy()),
                toml_str(&format!("{}; claude", env_line)),
            );
            fs::write(dir.join(format!("{}.toml", s)), toml).map_err(|e| e.to_string())?;
            let url = format!("warp://tab_config/{}{}", s, if l.new_window { "?new_window=true" } else { "" });
            let mut c = Command::new("explorer.exe");
            c.arg(url);
            spawn(c)
        }
        "tabby" => {
            let pname = format!("Claude · {}", l.name);
            let changed = ensure_tabby_profile(&tabby_config(), l, cfg, folder, &pname, "powershell.exe", &["-NoLogo", "-NoExit", "-Command", "claude"])?;
            if changed && tabby_running() {
                std::thread::sleep(std::time::Duration::from_millis(900)); // que Tabby recargue el config
            }
            let mut c = Command::new(tabby_exe());
            c.arg("profile").arg(&pname).current_dir(folder);
            spawn(c)
        }
        "wt" => {
            ensure_wt_fragment();
            // wt usa ';' para separar sus propios comandos: se escapa con '\;'.
            let ps = format!("{}\\; claude", env_line);
            let mut c = Command::new(wt_exe());
            c.raw_arg(if l.new_window { "-w new" } else { "-w 0" })
                .raw_arg("nt -p \"Claude Code\"")
                .raw_arg(format!("--title \"{}\" --suppressApplicationTitle", l.name.replace('"', "")))
                .raw_arg(format!("--tabColor \"{}\"", l.color))
                .raw_arg(format!("-d \"{}\"", folder.display()))
                .raw_arg(format!("powershell.exe -NoLogo -NoExit -Command \"{}\"", ps.replace('"', "\\\"")));
            spawn(c)
        }
        _ => {
            let mut c = Command::new("powershell.exe");
            c.arg("-NoLogo").arg("-NoExit").arg("-Command").arg(format!("$Host.UI.RawUI.WindowTitle={}; claude", ps_q(&title)));
            match cfg {
                Some(p) => c.env("CLAUDE_CONFIG_DIR", p),
                None => c.env_remove("CLAUDE_CONFIG_DIR"),
            };
            // Consola propia: sin redirigir stdin, o PowerShell lee EOF y se cierra.
            c.current_dir(folder).creation_flags(CREATE_NEW_CONSOLE);
            c.spawn().map(|_| ()).map_err(|e| e.to_string())
        }
    }
}

#[cfg(target_os = "macos")]
fn platform_launch_code(l: &CodeLaunch, cfg: Option<&Path>, folder: &Path) -> Result<(), String> {
    let env_line = match cfg {
        Some(c) => format!("export CLAUDE_CONFIG_DIR={}", sh_q(&c.to_string_lossy())),
        None => "unset CLAUDE_CONFIG_DIR".to_string(),
    };
    let line = format!("cd {} && {}; clear; claude", sh_q(&folder.to_string_lossy()), env_line);
    let osa_q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    match l.terminal.as_str() {
        "iterm" => {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
            let cmd = format!("{} -l -c {}", shell, sh_q(&format!("{}; exec {} -l", line, shell)));
            let script = if l.new_window {
                format!("tell application \"iTerm\"\nactivate\ncreate window with default profile command {}\nend tell", osa_q(&cmd))
            } else {
                format!(
                    "tell application \"iTerm\"\nactivate\nif (count of windows) = 0 then\ncreate window with default profile command {c}\nelse\ntell current window to create tab with default profile command {c}\nend if\nend tell",
                    c = osa_q(&cmd)
                )
            };
            let mut c = Command::new("/usr/bin/osascript");
            c.arg("-e").arg(script);
            spawn(c)
        }
        "warp" => {
            let dir = home().join(".warp/launch_configurations");
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let file = format!("{}.yaml", slug(l.name));
            let yaml = format!(
                "---\nname: {}\nwindows:\n  - tabs:\n      - title: {}\n        color: {}\n        layout:\n          cwd: {}\n          commands:\n            - exec: {}\n",
                yaml_str(&format!("Claude Code · {}", l.name)),
                yaml_str(l.name),
                closest_named(&l.color),
                yaml_str(&folder.to_string_lossy()),
                yaml_str(&format!("{}; claude", env_line)),
            );
            fs::write(dir.join(&file), yaml).map_err(|e| e.to_string())?;
            let mut c = Command::new("/usr/bin/open");
            c.arg(format!("warp://launch/{}", file));
            spawn(c)
        }
        "tabby" => {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
            let pname = format!("Claude · {}", l.name);
            let inner = format!("claude; exec {} -l", shell);
            let changed = ensure_tabby_profile(&tabby_config(), l, cfg, folder, &pname, &shell, &["-l", "-c", &inner])?;
            if changed && tabby_running() {
                std::thread::sleep(std::time::Duration::from_millis(900));
            }
            let mut c = Command::new("/usr/bin/open");
            c.arg("-a").arg("Tabby").arg("--args").arg("profile").arg(&pname);
            spawn(c)
        }
        _ => {
            let script = format!("tell application \"Terminal\"\nactivate\ndo script {}\nend tell", osa_q(&line));
            let mut c = Command::new("/usr/bin/osascript");
            c.arg("-e").arg(script);
            spawn(c)
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_launch_code(_: &CodeLaunch, _: Option<&Path>, _: &Path) -> Result<(), String> {
    Err("Plataforma no soportada".into())
}

// ---- Tabby

fn tabby_config() -> PathBuf {
    #[cfg(windows)]
    return dirs::config_dir().unwrap_or_default().join(r"tabby\config.yaml");
    #[cfg(not(windows))]
    return dirs::config_dir().unwrap_or_default().join("tabby/config.yaml");
}

fn tabby_running() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, sysinfo::ProcessRefreshKind::nothing());
    sys.processes().values().any(|p| {
        let n = p.name().to_string_lossy().to_lowercase();
        n == "tabby.exe" || n == "tabby"
    })
}

/// Mete (o actualiza) el perfil "claude-perfiles-<slug>" dentro de `profiles:` del config.yaml
/// de Tabby sin tocar los demás. Devuelve true si cambió el archivo.
fn ensure_tabby_profile(path: &Path, l: &CodeLaunch, cfg: Option<&Path>, folder: &Path, pname: &str, command: &str, args: &[&str]) -> Result<bool, String> {
    let id = format!("local:custom:claude-perfiles-{}", slug(l.name));
    let mut item = vec![
        "  - type: local".to_string(),
        format!("    name: {}", yaml_str(pname)),
        "    group: 'Claude Perfiles'".to_string(),
        "    icon: fas fa-terminal".to_string(),
        format!("    color: {}", yaml_str(&l.color)),
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

    let original = fs::read_to_string(path).unwrap_or_else(|_| "version: 8\nprofiles: []\n".into()).replace("\r\n", "\n");
    let mut lines: Vec<String> = original.split('\n').map(|s| s.to_string()).collect();
    let start = lines.iter().position(|l| l.starts_with("profiles:"));
    let (start, end, mut kept) = match start {
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
                    kept.extend(cur.drain(..));
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
    if result == original {
        return Ok(false);
    }
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).ok();
    }
    if path.is_file() {
        let bak = path.with_extension("yaml.bak-claudeperfiles");
        if !bak.exists() {
            fs::copy(path, bak).ok();
        }
    }
    fs::write(path, result).map_err(|e| e.to_string())?;
    Ok(true)
}

// ---- Windows Terminal: perfil «Claude Code» con un tema cálido (fragment, no toca settings.json)

#[cfg(windows)]
fn ensure_wt_fragment() {
    let dir = dirs::data_local_dir().unwrap_or_default().join(r"Microsoft\Windows Terminal\Fragments\ClaudePerfiles");
    let f = dir.join("claude.json");
    if f.is_file() {
        return;
    }
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&f, WT_FRAGMENT);
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

#[allow(dead_code)]
#[cfg(windows)]
pub fn no_window(c: &mut Command) {
    c.creation_flags(CREATE_NO_WINDOW);
}
