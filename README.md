# Claude Perfiles

Usá varias cuentas de Claude a la vez en la misma computadora. Por ejemplo, la del trabajo y la personal, o una por cliente.

Cada perfil tiene su propio login en **Claude Desktop** y en **Claude Code**. Podés tener dos o tres Desktop abiertos en paralelo, cada uno con su cuenta, y abrir Code en la terminal que prefieras con la sesión de ese perfil.

Funciona en **Windows 10/11** y **macOS 11 o más nuevo**. Es gratis y de código abierto (MIT). No es una app oficial de Anthropic.

## Descargar

Bajala de la [página de Releases](../../releases/latest):

- **Windows:** `ClaudePerfiles-Windows.exe`
- **Mac** (Intel y Apple Silicon): `ClaudePerfiles-Mac.dmg`

### La primera vez

La app todavía no tiene firma digital, que cuesta plata. Por eso el sistema avisa una sola vez:

- **Windows:** si aparece "Windows protegió su PC", tocá **Más información** y después **Ejecutar de todas formas**. Se instala en tu usuario y no pide permisos de administrador.
- **Mac:** arrastrá la app a Aplicaciones. La primera vez abrila con **clic derecho, Abrir**. Si macOS no te deja, andá a **Ajustes del Sistema, Privacidad y seguridad** y tocá **Abrir de todos modos**.

## Cómo se usa

1. **Nuevo perfil.** Elegí un nombre y un color.
2. **Desktop, Abrir.** Se abre un Claude Desktop aparte. Iniciá sesión con la cuenta de ese perfil.
3. **Code, Abrir.** Se abre una terminal con Claude Code. La primera vez escribí `/login`.

El perfil **Principal** es tu instalación de siempre y la app no lo toca.

Abajo a la izquierda elegís en qué terminal se abre Code:

| Windows | Mac |
|---|---|
| Windows Terminal, Warp, Tabby, PowerShell | Terminal, iTerm2, Warp, Tabby |

En el menú `···` de cada perfil podés cambiar la carpeta donde arranca Code, cambiar el color, crear atajos en el Escritorio (solo Windows), abrir la carpeta de datos y borrar el perfil.

## Cómo funciona

Nada raro, y por eso no tendría que molestar al antivirus:

- **Desktop:** cada perfil abre Claude Desktop con `--user-data-dir=<carpeta del perfil>`, que es una opción estándar de las apps Electron. Cada carpeta tiene su propia sesión.
- **Code:** cada perfil usa su propia `CLAUDE_CONFIG_DIR`, la variable oficial de Claude Code para separar configuraciones.
- Para saber qué Desktop está abierto, la app lee la lista de procesos `Claude` y nada más.
- No manda datos a ningún lado, no tiene telemetría y no se conecta a internet.

Dónde guarda las cosas:

- Windows: `%LOCALAPPDATA%\ClaudeProfiles\<perfil>\{desktop,code}`
- Mac: `~/Library/Application Support/ClaudeProfiles/<perfil>/{desktop,code}`

Para desinstalar en Windows, usá *Agregar o quitar programas*. En Mac, borrá la app. Si además querés borrar los perfiles, eliminá esa carpeta.

## Límites

- Los conectores MCP de Claude Desktop (`claude_desktop_config.json`) se comparten entre perfiles. Los de Claude Code van por perfil.
- La app no puede leer con qué mail estás logueado en Desktop, solo en Code.

## Compilar

Necesitás [Node 20+](https://nodejs.org) y [Rust](https://rustup.rs).

```
npm install
npm run dev      # modo desarrollo
npm run build    # instalador en src-tauri/target/release/bundle
```

Está hecha con [Tauri 2](https://tauri.app). El backend está en Rust (`src-tauri/src`) y la interfaz es HTML, CSS y JS sin frameworks (`ui/`). Cada tag `vX.Y.Z` compila los instaladores en GitHub Actions y los publica en Releases.
