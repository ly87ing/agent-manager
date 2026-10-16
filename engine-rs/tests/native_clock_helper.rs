//! Invalid helper requests must exit before application state or OS mutation.
use std::{fs, process::Command};

#[test]
fn helper_entry_points_reject_malformed_and_expired_requests_without_creating_state() {
    let root = std::env::temp_dir().join(format!("am-clock-entry-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    for executable in [
        env!("CARGO_BIN_EXE_agent-manager-engine"),
        env!("CARGO_BIN_EXE_agent-manager-clock-helper"),
    ] {
        for args in [
            vec!["--environment-clock-helper"],
            vec!["--environment-ntp-helper"],
            vec![
                "--environment-ntp-helper",
                "time.example",
                "2",
                "1",
                "1000",
                "0",
                "--confirm",
            ],
            vec![
                "--environment-ntp-helper",
                "https://time.example",
                "2",
                "1",
                "1000",
                "0",
                "--confirm",
            ],
            vec![
                "--environment-clock-helper",
                "example.invalid",
                "2000",
                "0",
                "--confirm",
            ],
            vec![
                "--environment-clock-helper",
                "192.0.2.1",
                "2000",
                "0",
                "--confirm",
            ],
        ] {
            let result = Command::new(executable)
                .args(&args)
                .env("HOME", root.join("home"))
                .env("USERPROFILE", root.join("home"))
                .env("AGENT_MANAGER_DATA_DIR", root.join("data"))
                .output()
                .unwrap();
            let expected =
                if args.len() == 7 && args[1] == "time.example" && cfg!(target_os = "linux") {
                    74
                } else if args.len() == 5
                    && args[1] == "192.0.2.1"
                    && cfg!(any(target_os = "linux", windows))
                {
                    74
                } else {
                    2
                };
            assert_eq!(result.status.code(), Some(expected), "{result:?}");
            assert!(
                result.stdout.is_empty() && result.stderr.is_empty(),
                "{result:?}"
            );
            assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        }
    }
    fs::remove_dir(root).unwrap();
}
