use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn command(root: &Path, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .env("AM_CAPTURE_MODE", mode)
        .env("AM_CAPTURE_ROOT", root);
    command
}

#[cfg(windows)]
fn close_inherited_pipes() {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    }
    // NUL stdio alone still inherits other inheritable handles.
    for handle in [
        std::io::stdout().as_raw_handle(),
        std::io::stderr().as_raw_handle(),
    ] {
        assert_ne!(unsafe { SetHandleInformation(handle, 1, 0) }, 0);
    }
}

fn main() {
    let mode = std::env::var("AM_CAPTURE_MODE").unwrap();
    let root = std::path::PathBuf::from(std::env::var_os("AM_CAPTURE_ROOT").unwrap());
    match mode.as_str() {
        "stdout" | "stderr" | "delayed-stdout" | "detached" => {
            #[cfg(windows)]
            if mode == "detached" {
                close_inherited_pipes();
            }
            let mut command = command(&root, "hold");
            command.stdin(Stdio::null());
            if mode == "detached" {
                command.stdout(Stdio::null()).stderr(Stdio::null());
            } else if mode != "stderr" {
                command.stdout(Stdio::inherit()).stderr(Stdio::null());
            } else {
                command.stdout(Stdio::null()).stderr(Stdio::inherit());
            }
            let _descendant = command.spawn().unwrap();
            let start = Instant::now();
            while !root.join("ready").exists() {
                assert!(start.elapsed() < Duration::from_secs(5));
                std::thread::sleep(Duration::from_millis(10));
            }
            if mode == "delayed-stdout" {
                std::thread::sleep(Duration::from_secs(2));
            }
            std::fs::write(root.join("parent-exited"), "done").unwrap();
        }
        "hold" => {
            std::fs::write(root.join("ready"), "ready").unwrap();
            std::thread::sleep(Duration::from_secs(5));
            std::fs::write(root.join("late"), "survived").unwrap();
        }
        "large-output" => {
            let stdout = [b'o'; 8192];
            let stderr = [b'e'; 8192];
            for _ in 0..640 {
                std::io::stdout().write_all(&stdout).unwrap();
                std::io::stderr().write_all(&stderr).unwrap();
            }
        }
        "output" => {
            println!("retained standard output");
            for line in 0..10 {
                eprintln!("diagnostic-{line}");
            }
            std::process::exit(7);
        }
        _ => panic!("Unknown capture fixture mode"),
    }
}
