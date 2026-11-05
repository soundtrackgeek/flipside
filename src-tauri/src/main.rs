#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if flipside_lib::run_cli_from_args() {
        return;
    }
    flipside_lib::run();
}
