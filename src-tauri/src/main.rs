// Esconde o console no Windows em build release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    localai_studio_lib::run()
}
