# Pruebas

Estado al 19/11/2026, versión 2.1.0.

- **probado**: se corrió de verdad y se miró el resultado (proceso creado, ventana, captura).
- **simulado**: probado en una compu real, pero con la terminal "ocultada" con `CP_HIDE_TERMINALS` (solo existe en builds de desarrollo) en vez de desinstalarla.
- **CI**: lo corre GitHub Actions en una Mac o un Windows limpios en cada push.
- **pendiente en Mac**: no hay Mac a mano. Abajo están los pasos exactos para probarlo.

## Tests automáticos

`cargo test --lib` (en Windows con `CP_TEST_MANIFEST=1`). Cubren:

- La resolución de la terminal guardada: instalada, desinstalada o desconocida, en Windows y Mac.
- Las comillas para PowerShell, sh y AppleScript, con rutas que tienen espacios, tildes, `ñ`, `'` y `"`.
- El escape de `;` para Windows Terminal.
- Que el merge del perfil de Tabby no toque los otros perfiles y sea idempotente.
- La expansión de `%VARIABLES%` del PATH.
- Que toda terminal que puede faltar tenga una forma de instalarse.

Solo en Mac (CI, `cargo test --lib -- --ignored mac_`):

- Que `osacompile` acepte los AppleScript de Terminal y de iTerm2.
- Que la detección real encuentre Terminal.app y el iTerm2 instalado por Homebrew.

## Windows (Windows 11, probado en la PC de Gerar)

| Caso | Estado | Cómo |
|---|---|---|
| Detecta Desktop abiertos y lee el mail de cada cuenta de Code | probado | captura |
| Code en Windows Terminal, perfil Principal | probado | `claude.exe ← powershell ← WindowsTerminal`, sin `CLAUDE_CONFIG_DIR` |
| Code en Windows Terminal, perfil con espacio ("Gerar Trabajo") | probado | `CLAUDE_CONFIG_DIR='…\Gerar Trabajo\code'` |
| Code en PowerShell suelto | probado | `claude.exe ← powershell` |
| Terminal guardada (Warp) no instalada → aviso arriba, Code abre en la del sistema, botón "Elegir terminal" | simulado | captura |
| Lo mismo, lanzado desde un atajo o la bandeja → aviso dentro de la app | simulado | captura |
| Windows Terminal con la app abierta y con la app cerrada (atajo) | probado | fallado y corregido (ver abajo) |
| Atajo con la app cerrada y terminal faltante → diálogo del sistema, Code abre igual y la app se cierra al aceptar | probado | captura |
| Si la terminal elegida no arranca → abre en PowerShell **avisando** | simulado | captura |
| Menú: las que faltan aparecen con "Instalar … · con winget" | simulado | captura |
| `winget install` de los 4 paquetes (`Anthropic.ClaudeCode`, `Microsoft.WindowsTerminal`, `Warp.Warp`, `Eugeny.Tabby`) | probado | `winget show` encuentra los 4; el install de WT corrido de verdad |
| Ventana visible de instalación desde la app (winget corre y la ventana queda abierta) | probado | con `--install wt` (flag solo de desarrollo) |
| Aviso "No encuentro Claude Code" con botón Instalar | probado | captura con datos de prueba |
| Code en Warp (build de release 2.1.0, app cerrada) | probado | `claude.exe ← powershell ← warp.exe`; TOML con `shell = "powershell"` |
| Code en Tabby (build de release 2.1.0) | probado | `claude.exe ← powershell ← Tabby.exe` |
| Primer arranque sin configuración | probado | toma los perfiles que ya existían y elige Windows Terminal |
| Carpeta de Code en la raíz de un disco (`D:\`) | test | la barra final se duplica para no romper la comilla |
| Instalador de release (el `.exe` de CI, MSVC) | probado | se instala sin admin, abre y lanza Code; Avast no lo bloqueó ni lo mandó a cuarentena |

**Fallado y corregido:** desde la app (con WebView2 cargado) no se puede activar `wt.exe`, que es un alias de app de la Store. CreateProcess da error 1920 y ShellExecute da error 5 con la app abierta, o devuelve OK sin abrir nada si la app se cierra enseguida. Pasa igual con una copia limpia del exe, así que no es un shim de compatibilidad. Ahora se lanza con un `cmd /c start` hijo, que anda en los dos casos. Si falla, abre PowerShell y avisa. Probado con la app abierta y con la app cerrada (atajo del Escritorio).

## Mac: pendiente de validación manual en Mac

Lo que sí corre en CI: compilación universal (Intel + Apple Silicon), tests, AppleScript que compila, detección de Terminal e iTerm2, `NSAppleEventsUsageDescription` en el Info.plist y firma ad-hoc.

Pasos para quien tenga una Mac (unos 15 minutos):

1. Bajá el `.dmg` del último release, o el artefacto `instalador-macOS` del último run de CI. Arrastrá la app a Aplicaciones.
2. **Gatekeeper.** Abrila. Tiene que aparecer "no se puede abrir". Andá a Ajustes del Sistema → Privacidad y seguridad → *Abrir de todos modos* → contraseña. Tiene que abrir. Anotá la versión de macOS.
3. **Terminal.app.** Elegí Terminal abajo a la izquierda. Abrí Code del Principal:
   - La primera vez macOS pregunta "Claude Perfiles quiere controlar Terminal". Tocá *Permitir*.
   - Con Terminal cerrada: se abre **una** ventana (no dos) que corre `claude`.
   - Con Terminal abierta: se abre una ventana nueva.
4. **Permiso denegado.** En Ajustes → Privacidad y seguridad → Automatización, apagá Terminal debajo de Claude Perfiles. Abrí Code: tiene que salir el mensaje en castellano que explica dónde activarlo (no un error en inglés).
5. **Perfil nuevo.** Creá "Prueba Ñ" y abrí su Code. En la terminal: `echo $CLAUDE_CONFIG_DIR` tiene que mostrar `…/ClaudeProfiles/Prueba Ñ/code`. Con `/login`, entrá con otra cuenta. En el Principal, `echo $CLAUDE_CONFIG_DIR` tiene que estar vacío.
6. **iTerm2** (si no está, probá el *Instalar* del menú, que usa Homebrew si lo tenés):
   - iTerm cerrado → una sola ventana con claude.
   - iTerm abierto + "Pestaña nueva" → pestaña nueva.
   - iTerm abierto + "Ventana nueva" → ventana nueva.
7. **Warp.** Abrí Code: tiene que abrir una pestaña "Claude Code · <perfil>" con el color del perfil, en la carpeta y con claude corriendo. Probá también "Ventana nueva". El archivo queda en `~/.warp/tab_configs/claude-<perfil>.toml`.
8. **Tabby.** Lo mismo. Revisá que en `~/Library/Application Support/tabby/config.yaml` estén tus otros perfiles intactos.
9. **Desinstalar la terminal elegida.** Elegí Warp, cerrá la app, borrá Warp.app y volvé a abrir la app. Tiene que aparecer el aviso arriba, Code tiene que abrir en Terminal y el menú tiene que ofrecer *Instalar Warp*.
10. **Sin Claude Code.** En un usuario de macOS sin Claude Code, tiene que aparecer el aviso con *Instalar Claude Code*. Al tocarlo se abre Terminal con el instalador oficial, y al terminar el aviso tiene que desaparecer solo.
11. **Desktop.** Abrí Desktop en dos perfiles. Tienen que quedar dos Claude abiertos, cada uno con su login.

Si algo falla, una captura y la versión de macOS alcanzan para arreglarlo.

## Riesgos abiertos

- La Mac no está validada a mano (arriba están los pasos).
- La app no está firmada: SmartScreen y Gatekeeper avisan la primera vez. La firma (Apple USD 99 por año, Windows con certificado) es una decisión pendiente.
- Warp en Windows: el comando usa sintaxis de PowerShell y el TOML pide `shell = "pwsh"` o `"powershell"`. Si Warp no respeta `shell`, depende de que la shell por defecto de Warp sea PowerShell.
- Sin winget, la app no corre `irm … | iex` (un antivirus lo vería como un descargador): abre la guía oficial de instalación.
- Antivirus: el instalador se probó bajado por `gh` e instalado en silencio, y Avast no lo marcó. Falta el camino de un colega: bajarlo con el navegador (Mark of the Web, SmartScreen) e instalarlo con el asistente.
