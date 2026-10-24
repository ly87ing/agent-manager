#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    std::process::exit(agent_manager_engine::claude_data_inspection_helper_cli(
        &args,
    ));
}
