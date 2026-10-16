#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(agent_manager_engine::environment_clock_helper_cli(&args).unwrap_or(2));
}
