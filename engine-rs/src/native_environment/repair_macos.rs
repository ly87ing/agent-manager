use super::{
    repair::{Item, Settings, Snapshot},
    Target,
};
use anyhow::{bail, Result};
use fs2::FileExt;
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    process::Stdio,
    time::Duration,
};

pub(super) struct MacSettings;

pub(super) fn validate_runtime_locale(target: &Target) -> Result<()> {
    if target
        .languages
        .iter()
        .any(|language| !language.as_bytes()[0].is_ascii_alphabetic())
    {
        bail!("Language tags for repair must start with a letter.");
    }
    if let Some(locale) = &target.locale {
        let available = super::macos::command("/usr/bin/locale", &["-a"]).is_some_and(|values| {
            values
                .lines()
                .any(|value| super::locale_key(value) == super::locale_key(locale))
        });
        if !available {
            bail!("The process locale is unavailable on this Mac. Choose a supported locale before repair.");
        }
        let language = format!("LANG={locale}");
        let all = format!("LC_ALL={locale}");
        let output = super::macos::command(
            "/usr/bin/env",
            &[
                "-i",
                "PATH=/usr/bin:/bin",
                &language,
                &all,
                "/usr/bin/locale",
            ],
        );
        if !output.is_some_and(|output| {
            super::LOCALE_CATEGORIES.iter().all(|category| {
                output.lines().any(|line| {
                    line.strip_prefix(&format!("{category}="))
                        .is_some_and(|value| {
                            super::locale_key(value.trim_matches('"')) == super::locale_key(locale)
                        })
                })
            })
        }) {
            bail!("The requested process locale did not become effective in an isolated locale check.");
        }
    }
    Ok(())
}

pub(super) fn mutation_lock() -> Result<File> {
    let uid = unsafe { libc::geteuid() };
    let path = format!("/tmp/io.github.ly87ing.agent-manager.environment-{uid}.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        bail!("The environment repair lock is not a private regular file.");
    }
    file.try_lock_exclusive().map_err(|_| {
        anyhow::anyhow!(
            "An environment repair or time correction is already running for this user."
        )
    })?;
    Ok(file)
}

fn session_keys(target: &Target) -> Vec<&'static str> {
    let mut keys = Vec::new();
    if target.timezone.is_some() {
        keys.push("TZ");
    }
    if target.locale.is_some() {
        keys.extend(["LANG", "LC_ALL"]);
        keys.extend(super::LOCALE_CATEGORIES);
    }
    keys
}

fn launchctl(args: &[&str], limit: usize) -> Option<String> {
    let mut command = crate::process::launch_command("/bin/launchctl");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = crate::process::capture_group(&mut command, Duration::from_secs(3), limit).ok()?;
    if !output.status.success() || output.stdout.len() >= limit {
        return None;
    }
    launchctl_value(output.stdout)
}

fn launchctl_value(output: Vec<u8>) -> Option<String> {
    String::from_utf8(output)
        .ok()
        .map(|value| value.strip_suffix('\n').unwrap_or(&value).to_string())
}

#[repr(C)]
struct AuditToken {
    words: [u32; 8],
}

#[link(name = "bsm")]
unsafe extern "C" {
    fn audit_token_to_asid(token: AuditToken) -> libc::pid_t;
}

fn current_session_id() -> Option<libc::pid_t> {
    let mut token = AuditToken { words: [0; 8] };
    let mut count = 8;
    // TASK_AUDIT_TOKEN and its eight-word ABI are defined in mach/task_info.h.
    #[allow(deprecated)]
    let result = unsafe {
        libc::task_info(
            libc::mach_task_self(),
            15,
            token.words.as_mut_ptr().cast(),
            &mut count,
        )
    };
    if result != 0 || count != 8 {
        return None;
    }
    let asid = unsafe { audit_token_to_asid(token) };
    (asid > 0).then_some(asid)
}

fn graphical_session() -> Option<String> {
    let uid = unsafe { libc::geteuid() };
    if launchctl(&["managername"], 1024).as_deref() != Some("Aqua")
        || launchctl(&["manageruid"], 1024)?.parse::<u32>().ok()? != uid
    {
        return None;
    }
    let gui = format!("gui/{uid}");
    let asid = current_session_id()?;
    let login = format!("login/{asid}");
    let mut command = crate::process::launch_command("/bin/launchctl");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args(["print", &login])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Check domain existence without reading any launchd environment contents.
    crate::process::capture_group(&mut command, Duration::from_secs(3), 1024)
        .ok()?
        .status
        .success()
        .then(|| format!("{gui} · {login} · launchd Aqua"))
}

impl Settings for MacSettings {
    fn snapshot(&mut self, target: &Target) -> Result<Snapshot> {
        let observed = super::macos::collect();
        let mut snapshot = Snapshot::default();
        for (id, value) in [
            ("time.timezone", observed.timezone.map(|value| json!(value))),
            (
                "time.automatic_timezone",
                observed.automatic_timezone.map(|value| json!(value)),
            ),
            ("ntp.server", observed.ntp_server.map(|value| json!(value))),
            (
                "ntp.automatic_time",
                observed.automatic_time.map(|value| json!(value)),
            ),
            (
                "language.preferred",
                observed.languages.map(|value| json!(value)),
            ),
            (
                "language.region",
                observed.apple_locale.map(|value| json!(value)),
            ),
        ] {
            snapshot.values.insert(id.into(), value);
        }
        snapshot.session_scope = format!("uid:{} · launchd session unavailable", unsafe {
            libc::geteuid()
        });
        if let Some(scope) = graphical_session() {
            snapshot.session_scope = scope;
            snapshot.session_available = true;
            for key in session_keys(target) {
                snapshot.values.insert(
                    format!("session.{key}"),
                    launchctl(&["getenv", key], 16 * 1024).map(|value| json!(value)),
                );
            }
        }
        Ok(snapshot)
    }

    fn system(&mut self, items: &[Item]) -> Result<(String, Vec<Item>, Option<bool>)> {
        let script = system_script(items)?;
        let authorization = r#"on run argv
try
    return do shell script "/usr/bin/env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin LC_ALL=C /bin/sh -c " & quoted form of (item 1 of argv) with administrator privileges
on error messageText number errorNumber
    if errorNumber is -128 then return "cancelled"
    return "failed"
end try
end run"#;
        let mut command = crate::process::launch_command("/usr/bin/osascript");
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LC_ALL", "C")
            .args(["-e", authorization, &script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output =
            crate::process::capture_group(&mut command, Duration::from_secs(150), 16 * 1024);
        let text = output
            .ok()
            .filter(|output| output.status.success() && output.stdout.len() < 16 * 1024)
            .and_then(|output| String::from_utf8(output.stdout).ok());
        parse_system_result(items, text.as_deref())
    }

    fn write_user(&mut self, item: &Item) -> bool {
        let Some((program, args)) = user_command(item) else {
            return false;
        };
        let args: Vec<_> = args.iter().map(String::as_str).collect();
        if program == "/bin/launchctl" {
            return launchctl(&args, 16 * 1024).is_some();
        }
        super::macos::command(program, &args).is_some()
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn user_command(item: &Item) -> Option<(&'static str, Vec<String>)> {
    let strings = |values: &[&str]| {
        values
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>()
    };
    match item.id.as_str() {
        "language.preferred" => {
            let mut args = strings(&["write", "NSGlobalDomain", "AppleLanguages", "-array"]);
            args.extend(
                item.expected
                    .as_array()?
                    .iter()
                    .map(|value| value.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()?,
            );
            Some(("/usr/bin/defaults", args))
        }
        "language.region" => Some((
            "/usr/bin/defaults",
            strings(&[
                "write",
                "NSGlobalDomain",
                "AppleLocale",
                "-string",
                item.expected.as_str()?,
            ]),
        )),
        id if id.starts_with("session.") => {
            let key = id.strip_prefix("session.")?;
            if !["TZ", "LANG", "LC_ALL"].contains(&key) && !super::LOCALE_CATEGORIES.contains(&key)
            {
                return None;
            }
            Some((
                "/bin/launchctl",
                strings(&["setenv", key, item.expected.as_str()?]),
            ))
        }
        _ => None,
    }
}

fn system_value(item: &Value) -> Option<String> {
    match item {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(if *value { "1" } else { "0" }.into()),
        _ => None,
    }
}

fn system_commands(item: &Item) -> Result<(String, String)> {
    let value =
        system_value(&item.expected).ok_or_else(|| anyhow::anyhow!("Invalid system target."))?;
    let (read, write) = match item.id.as_str() {
        "time.timezone" => (
            "/usr/bin/readlink /etc/localtime | /usr/bin/sed -n 's|^.*[/]zoneinfo/||p'".into(),
            format!("/usr/sbin/systemsetup -settimezone {}", quote(&value))),
        "time.automatic_timezone" => (
            "/usr/bin/defaults read /Library/Preferences/com.apple.timezone.auto Active".into(),
            format!("/usr/bin/defaults write /Library/Preferences/com.apple.timezone.auto Active -bool {}", if value == "1" { "true" } else { "false" })),
        "ntp.server" => (
            "/usr/sbin/systemsetup -getnetworktimeserver | /usr/bin/sed -n 's/^Network Time Server: //p' | /usr/bin/tr '[:upper:]' '[:lower:]' | /usr/bin/sed 's/\\.$//'".into(),
            format!("/usr/sbin/systemsetup -setnetworktimeserver {}", quote(&value))),
        "ntp.automatic_time" => (
            "/usr/sbin/systemsetup -getusingnetworktime | /usr/bin/sed -n -e 's/^Network Time: On$/1/p' -e 's/^Network Time: Off$/0/p'".into(),
            format!("/usr/sbin/systemsetup -setusingnetworktime {}", if value == "1" { "on" } else { "off" })),
        _ => bail!("Unsupported system repair item."),
    };
    Ok((read, write))
}

fn system_script(items: &[Item]) -> Result<String> {
    let mut script = String::from(
        "PATH=/usr/bin:/bin:/usr/sbin:/sbin; export PATH; LC_ALL=C; export LC_ALL\nblocked=0\n",
    );
    for item in items {
        let (read, write) = system_commands(item)?;
        let expected = quote(&system_value(&item.expected).unwrap());
        let before = item.before.as_ref().and_then(system_value);
        let baseline = before.map_or("true".into(), |value| {
            format!("[ \"$actual\" = {} ]", quote(&value))
        });
        script.push_str(&format!("if [ \"$blocked\" = 1 ]; then status=not_applied\nelif actual=$({read}) && [ -n \"$actual\" ]; then\n  if [ \"$actual\" = {expected} ]; then status=unchanged\n  elif {baseline}; then\n    command_ok=1; {write} >/dev/null 2>&1 || command_ok=0\n    if actual=$({read}) && [ \"$actual\" = {expected} ]; then\n      if [ \"$command_ok\" = 1 ]; then status=verified; else status=verified_after_error; blocked=1; fi\n    else status=outcome_unknown; blocked=1; fi\n  else status=stale; blocked=1; fi\nelse status=unavailable; blocked=1; fi\n/usr/bin/printf '%s\\t%s\\n' {} \"$status\"\n", quote(&item.id)));
    }
    script.push_str("automatic=$(/usr/sbin/systemsetup -getusingnetworktime | /usr/bin/sed -n -e 's/^Network Time: On$/1/p' -e 's/^Network Time: Off$/0/p'); /usr/bin/printf 'automatic_time\\t%s\\n' \"$automatic\"\n");
    Ok(script)
}

fn parse_system_result(
    items: &[Item],
    text: Option<&str>,
) -> Result<(String, Vec<Item>, Option<bool>)> {
    if text.map(str::trim) == Some("cancelled") {
        return Ok(("cancelled".into(), vec![], None));
    }
    let mut results = items.to_vec();
    let lines: Vec<_> = text
        .unwrap_or("")
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect();
    for item in &mut results {
        let values: Vec<_> = lines
            .iter()
            .filter(|(id, _)| *id == item.id)
            .map(|(_, value)| *value)
            .collect();
        item.status = if values.len() == 1
            && [
                "unchanged",
                "verified",
                "verified_after_error",
                "outcome_unknown",
                "stale",
                "not_applied",
                "unavailable",
            ]
            .contains(&values[0])
        {
            values[0].into()
        } else {
            "outcome_unknown".into()
        };
    }
    let automatic = lines
        .iter()
        .find(|(id, _)| *id == "automatic_time")
        .and_then(|(_, value)| match *value {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        });
    let status = if results
        .iter()
        .all(|item| matches!(item.status.as_str(), "unchanged" | "verified"))
    {
        "applied"
    } else {
        "partial"
    };
    Ok((status.into(), results, automatic))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str, before: Value, expected: Value) -> Item {
        Item {
            id: id.into(),
            scope: "system".into(),
            before: Some(before),
            expected,
            status: "change".into(),
        }
    }
    #[test]
    fn privileged_commands_are_fixed_quoted_and_check_before_and_after_each_write() {
        let items = vec![
            item(
                "time.timezone",
                json!("Asia/Singapore"),
                json!("America/Los_Angeles"),
            ),
            item(
                "ntp.server",
                json!("time.apple.com"),
                json!("time.nist.gov"),
            ),
            item("ntp.automatic_time", json!(false), json!(true)),
            item("time.automatic_timezone", json!(true), json!(false)),
        ];
        let script = system_script(&items).unwrap();
        assert!(script.contains("-settimezone 'America/Los_Angeles'"));
        assert!(script.contains("-setnetworktimeserver 'time.nist.gov'"));
        assert!(script.contains("-setusingnetworktime on"));
        assert!(script.contains("Active -bool false"));
        assert_eq!(script.matches("command_ok=1;").count(), 4);
        assert_eq!(script.matches("status=stale; blocked=1").count(), 4);
        assert!(!script.contains("sntp"));
        assert!(!script.contains("killall"));
        assert_eq!(quote("a'b"), "'a'\\''b'");
        let mut command = std::process::Command::new("/bin/sh");
        let mut child = command.args(["-n"]).stdin(Stdio::piped()).spawn().unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
    }
    #[test]
    fn incomplete_authorization_output_never_claims_success() {
        let items = vec![item("ntp.automatic_time", json!(false), json!(true))];
        assert_eq!(
            parse_system_result(&items, Some("cancelled\n")).unwrap().0,
            "cancelled"
        );
        assert_eq!(
            parse_system_result(&items, None).unwrap().1[0].status,
            "outcome_unknown"
        );
        assert_eq!(
            parse_system_result(
                &items,
                Some("ntp.automatic_time\tverified\nautomatic_time\t1\n")
            )
            .unwrap()
            .2,
            Some(true)
        );
        assert_eq!(
            parse_system_result(
                &items,
                Some("ntp.automatic_time\tverified\nntp.automatic_time\tverified\n")
            )
            .unwrap()
            .0,
            "partial"
        );
        assert!(user_command(&item("session.PATH", json!("old"), json!("new"))).is_none());
    }

    #[test]
    fn audit_session_identity_uses_the_documented_token_abi() {
        assert_eq!(std::mem::size_of::<AuditToken>(), 32);
        let token = AuditToken {
            words: [0, 0, 0, 0, 0, 0, 100002, 0],
        };
        assert_eq!(unsafe { audit_token_to_asid(token) }, 100002);
    }

    #[test]
    fn launchctl_values_preserve_whitespace_and_distinguish_empty_success_from_failure() {
        assert_eq!(
            launchctl_value(b" en_US.UTF-8 \n".to_vec()).as_deref(),
            Some(" en_US.UTF-8 ")
        );
        assert_eq!(launchctl_value(b"\n".to_vec()).as_deref(), Some(""));
        assert_eq!(launchctl_value(b"C\n\n".to_vec()).as_deref(), Some("C\n"));
        assert!(launchctl_value(vec![0xff]).is_none());
    }

    #[test]
    fn mutation_lock_is_shared_by_separate_open_handles_and_released_by_drop() {
        const CHILD: &str = "AM_ENVIRONMENT_LOCK_TEST_CHILD";
        // Other parallel tests must not fork while this lock descriptor is open.
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "native_environment::repair_macos::tests::mutation_lock_is_shared_by_separate_open_handles_and_released_by_drop",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            return;
        }
        let first = mutation_lock().unwrap();
        assert!(mutation_lock().is_err());
        drop(first);
        mutation_lock().expect("The environment mutation lock must be available after drop");
    }

    #[test]
    fn real_shell_replays_system_repair_and_stale_failure_against_disposable_settings() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("repair-shell-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&root).unwrap();
        let program = root.join("systemsetup");
        std::fs::write(&program, "#!/bin/sh\ncase \"$1\" in\n-getnetworktimeserver) printf 'Network Time Server: '; cat \"$REPAIR_FIXTURE/server\";;\n-setnetworktimeserver) printf '%s\\n' \"$2\" > \"$REPAIR_FIXTURE/server\"; printf 'write\\n' >> \"$REPAIR_FIXTURE/writes\";;\n-getusingnetworktime) echo 'Network Time: On';;\n*) exit 1;;\nesac\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let items = vec![item(
            "ntp.server",
            json!("old.example"),
            json!("new.example"),
        )];
        let script = system_script(&items)
            .unwrap()
            .replace("/usr/sbin/systemsetup", &quote(program.to_str().unwrap()));
        for (actual, status, writes) in [
            ("old.example", "verified", 1),
            ("new.example", "unchanged", 0),
            ("changed.example", "stale", 0),
        ] {
            std::fs::write(root.join("server"), format!("{actual}\n")).unwrap();
            std::fs::write(root.join("writes"), "").unwrap();
            let output = std::process::Command::new("/bin/sh")
                .args(["-c", &script])
                .env("REPAIR_FIXTURE", &root)
                .output()
                .unwrap();
            assert!(output.status.success());
            let text = String::from_utf8(output.stdout).unwrap();
            let result = parse_system_result(&items, Some(&text)).unwrap();
            assert_eq!(result.1[0].status, status);
            assert_eq!(
                std::fs::read_to_string(root.join("writes"))
                    .unwrap()
                    .lines()
                    .count(),
                writes
            );
            assert_eq!(
                std::fs::read_to_string(root.join("server")).unwrap().trim(),
                if writes == 1 { "new.example" } else { actual }
            );
            assert_eq!(result.2, Some(true));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
