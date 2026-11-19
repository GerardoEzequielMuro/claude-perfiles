// Sin consola en Windows (release).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    claude_perfiles_lib::run()
}
