//! Literal PowerShell recovery commands; metadata remains owned by the application.
use super::{Plan, Step};
use std::path::Path;

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn path(value: &Path) -> String {
    quote(&value.to_string_lossy())
}

fn guard_file(file: &Path) -> String {
    use sha2::{Digest, Sha256};
    let expected = match std::fs::symlink_metadata(file) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 64 * 1024 => {
            match std::fs::read(file) {
                Ok(bytes) => Some(format!("{:X}", Sha256::digest(bytes))),
                Err(_) => return "throw 'Cannot verify the existing command entry'".into(),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        _ => return "throw 'Cannot verify the existing command entry'".into(),
    };
    let check = expected.map(|digest| format!("if (-not [IO.File]::Exists({0}) -or (Get-FileHash -LiteralPath {0} -Algorithm SHA256 -ErrorAction Stop).Hash -cne '{digest}') {{ throw 'File changed after preview' }}", path(file)))
        .unwrap_or_else(|| format!("if (Test-Path -LiteralPath {}) {{ throw 'Destination appeared after preview' }}", path(file)));
    format!("if (Test-Path -LiteralPath {0}) {{ $item = Get-Item -LiteralPath {0} -Force -ErrorAction Stop; if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {{ throw 'Refusing a directory or linked file' }} }}; {check}", path(file))
}

fn parent(file: &Path) -> String {
    file.parent()
        .map(|parent| {
            format!(
                "[IO.Directory]::CreateDirectory({}) | Out-Null; ",
                path(parent)
            )
        })
        .unwrap_or_default()
}

// Check each existing ancestor immediately before a filesystem operation.
fn ancestors(file: &Path, include_leaf: bool) -> String {
    let paths = file
        .ancestors()
        .skip(usize::from(!include_leaf))
        .map(path)
        .collect::<Vec<_>>()
        .join(",");
    format!("foreach ($ancestor in @({paths})) {{ try {{ $a = Get-Item -LiteralPath $ancestor -Force -ErrorAction Stop; if ($a.Attributes -band [IO.FileAttributes]::ReparsePoint) {{ throw 'Refusing a linked path ancestor' }} }} catch {{ if ($_.CategoryInfo.Category -ne 'ObjectNotFound') {{ throw }} }} }}; ")
}

fn current_link(program_dir: &Path, version: &str) -> String {
    let current = program_dir.join("current");
    let target = program_dir.join(version);
    let stage = program_dir.join(format!(".current-{}", crate::store::uuid_v4()));
    let old = program_dir.join(format!(".previous-{}", crate::store::uuid_v4()));
    let before = crate::platform::read_link(&current).ok().map(|target| {
        if target.is_absolute() {
            target
        } else {
            program_dir.join(target)
        }
    });
    let check = if let Some(before) = &before {
        format!("$item = Get-Item -LiteralPath {0} -Force -ErrorAction Stop; $targets = @($item.Target); if (-not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $targets.Count -ne 1) {{ throw 'Current link changed after preview' }}; $actual = $targets[0]; if (-not [IO.Path]::IsPathRooted($actual)) {{ $actual = [IO.Path]::Combine({1}, $actual) }}; if ([IO.Path]::GetFullPath($actual) -ine {2}) {{ throw 'Current link changed after preview' }}; ", path(&current), path(program_dir), path(before))
    } else {
        format!("if (Get-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue) {{ throw 'Current link appeared after preview' }}; ", path(&current))
    };
    let switch = if before.is_some() {
        format!("[IO.Directory]::Move({0}, {1}); try {{ [IO.Directory]::Move({2}, {0}) }} catch {{ if (-not (Get-Item -LiteralPath {0} -Force -ErrorAction SilentlyContinue)) {{ [IO.Directory]::Move({1}, {0}) }}; throw }}; [IO.Directory]::Delete({1})", path(&current), path(&old), path(&stage))
    } else {
        format!("[IO.Directory]::Move({}, {})", path(&stage), path(&current))
    };
    // cmd expands these environment variables once; their values are never reparsed.
    // Unlike PowerShell 5.1 New-Item -Target, mklink does not glob bracketed paths.
    format!("{}{}{}if (-not [IO.Directory]::Exists({target})) {{ throw 'Version directory is missing' }}; $p = New-Object Diagnostics.ProcessStartInfo; $p.FileName = [IO.Path]::Combine([Environment]::SystemDirectory, 'cmd.exe'); $p.UseShellExecute = $false; $p.Arguments = '/d /v:OFF /c mklink /J \"%AM_JUNCTION_LINK%\" \"%AM_JUNCTION_TARGET%\"'; $p.EnvironmentVariables['AM_JUNCTION_LINK'] = {stage}; $p.EnvironmentVariables['AM_JUNCTION_TARGET'] = {target}; $c = [Diagnostics.Process]::Start($p); $c.WaitForExit(); if ($c.ExitCode -ne 0) {{ throw 'Cannot stage the new junction' }}; try {{ $item = Get-Item -LiteralPath {stage} -Force -ErrorAction Stop; if (@($item.Target).Count -ne 1 -or [IO.Path]::GetFullPath(@($item.Target)[0]) -ine {target}) {{ throw 'Staged junction target does not match' }}; {check}{switch} }} finally {{ if (Get-Item -LiteralPath {stage} -Force -ErrorAction SilentlyContinue) {{ [IO.Directory]::Delete({stage}) }} }}", ancestors(&current, false), ancestors(&target, true), ancestors(&stage, true), target=path(&target), stage=path(&stage))
}

pub(super) fn render(plan: &Plan) -> Vec<String> {
    plan.steps.iter().filter(|step| !plan.manual_exec_only || matches!(step, Step::Exec { .. })).filter_map(|step| {
        let command = match step {
            Step::Move { from, to } => format!("{}{}{}if (Test-Path -LiteralPath {to}) {{ throw 'Move destination already exists' }}; if ([IO.File]::Exists({from})) {{ [IO.File]::Move({from}, {to}) }} else {{ [IO.Directory]::Move({from}, {to}) }}", ancestors(from, false), ancestors(to, true), parent(to), from=path(from), to=path(to)),
            Step::Exec { program, args, env, cwd } => match crate::native_launch::windows_program_command(program, args, env, cwd.as_deref()) {
                Ok(command) => format!("{command}; if ($LASTEXITCODE -ne 0) {{ throw 'Program command failed; stop before continuing' }}"),
                Err(_) => "throw 'This command cannot be represented safely in PowerShell'".into(),
            },
            Step::Link { path: file, target, .. } => match crate::platform::cmd_shim_text(target) {
                Ok(text) => format!("{}{}{}; [IO.File]::WriteAllText({}, {}, [Text.UTF8Encoding]::new($false))", ancestors(file, true), parent(file), guard_file(file), path(file), quote(&text)),
                Err(_) => "throw 'This command entry cannot be represented safely in a batch file'".into(),
            },
            Step::Unlink { path: file } => format!("{}{}; Remove-Item -LiteralPath {} -Force -ErrorAction Stop", ancestors(file, true), guard_file(file), path(file)),
            Step::RemoveDir { path: directory } => format!("{guard}$pending = New-Object 'Collections.Generic.Stack[string]'; $pending.Push({0}); while ($pending.Count) {{ $dir = $pending.Pop(); foreach ($child in [IO.Directory]::EnumerateFileSystemEntries($dir)) {{ $attributes = [IO.File]::GetAttributes($child); if ($attributes -band [IO.FileAttributes]::ReparsePoint) {{ throw 'Refusing recursive removal through a link' }}; if ($attributes -band [IO.FileAttributes]::Directory) {{ $pending.Push($child) }} }} }}; [IO.Directory]::Delete({0}, $true)", path(directory), guard=ancestors(directory, true)),
            Step::SetCurrent { program_dir, version } => current_link(program_dir, version),
            Step::WriteMeta { .. } | Step::WriteBackupMeta { .. } | Step::SaveEntryDir { .. } => return None,
        };
        Some(format!("& {{ $ErrorActionPreference = 'Stop'; {command} }}"))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::super::Action;
    use super::*;
    fn plan(steps: Vec<Step>) -> Plan {
        Plan {
            program: "npm:sample".into(),
            action: Action::Install,
            target_version: None,
            steps,
            verify_bins: vec![],
            verify_version: None,
            notes: vec![],
            scripts_ignored: false,
            backup_dir: None,
            manual_exec_only: false,
        }
    }
    #[test]
    fn command_rendering_uses_literal_child_environment_working_directory_and_failure_gate() {
        let commands = render(&plan(vec![Step::Exec {
            program: "C:\\Program Files\\npm.cmd".into(),
            args: vec!["".into(), "literal %VALUE% & !value!".into()],
            env: vec![("YOUR_VAR".into(), "O'Brien".into())],
            cwd: Some("C:\\work space".into()),
        }]));
        let command = &commands[0];
        assert!(command.contains("$p.WorkingDirectory = 'C:\\work space'"));
        assert!(command.contains("$p.EnvironmentVariables['YOUR_VAR'] = 'O''Brien'"));
        assert!(command.contains("/v:OFF /d /c"));
        assert!(command.contains("$LASTEXITCODE -ne 0"));
    }
    #[cfg(windows)]
    fn execute(commands: &[String]) -> std::process::Output {
        let powershell = Path::new(&std::env::var("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        std::process::Command::new(powershell)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("$ErrorActionPreference='Stop'; {}", commands.join("; ")),
            ])
            .env("PSModuleAnalysisCachePath", "NUL")
            .output()
            .unwrap()
    }
    #[cfg(windows)]
    #[test]
    fn native_manual_commands_reject_changed_ancestors_and_preserve_current_on_staging_failure() {
        let root = std::env::temp_dir().join(format!("am-manual-{}", crate::store::uuid_v4()));
        let parent = root.join("bin");
        let outside = root.join("outside");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let entry = parent.join("entry.cmd");
        let commands = render(&plan(vec![Step::Link {
            path: entry,
            target: root.join("target.exe"),
            register: false,
        }]));
        std::fs::remove_dir(&parent).unwrap();
        junction::create(&outside, &parent).unwrap();
        assert!(!execute(&commands).status.success());
        assert!(!outside.join("entry.cmd").exists());
        std::fs::write(outside.join("sentinel"), "keep").unwrap();
        let remove = render(&plan(vec![Step::RemoveDir {
            path: parent.clone(),
        }]));
        assert!(!execute(&remove).status.success());
        assert_eq!(
            std::fs::read_to_string(outside.join("sentinel")).unwrap(),
            "keep"
        );
        let program = root.join("program [literal]");
        std::fs::create_dir_all(program.join("1.0")).unwrap();
        std::fs::create_dir_all(program.join("2.0")).unwrap();
        match std::os::windows::fs::symlink_dir("1.0", program.join("current")) {
            Ok(()) => assert_eq!(
                crate::platform::read_link(program.join("current")).unwrap(),
                Path::new("1.0")
            ),
            Err(error) => {
                eprintln!(
                    "Relative symlink branch unavailable: {error}; junction branch still executes"
                );
                junction::create(program.join("1.0"), program.join("current")).unwrap();
            }
        }
        let commands = render(&plan(vec![Step::SetCurrent {
            program_dir: program.clone(),
            version: "2.0".into(),
        }]));
        let re = regex::Regex::new(r"AM_JUNCTION_LINK'\] = '([^']+)'").unwrap();
        let stage = Path::new(re.captures(&commands[0]).unwrap().get(1).unwrap().as_str());
        std::fs::create_dir(stage).unwrap();
        std::fs::write(stage.join("foreign"), "keep").unwrap();
        assert!(!execute(&commands).status.success());
        assert_eq!(
            program.join("current").canonicalize().unwrap(),
            program.join("1.0").canonicalize().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(stage.join("foreign")).unwrap(),
            "keep"
        );
        std::fs::remove_dir_all(stage).unwrap();
        let output = execute(&commands);
        assert!(output.status.success(), "{:?}", output);
        assert_eq!(
            crate::platform::read_link(program.join("current")).unwrap(),
            program.join("2.0")
        );
        crate::platform::remove_link(parent).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn native_powershell_creates_literal_entries_and_junctions_in_disposable_paths() {
        let root = std::env::temp_dir().join(format!("am-manual-{}", crate::store::uuid_v4()));
        let program = root.join("program [literal]");
        let version = program.join("1.0");
        std::fs::create_dir_all(&version).unwrap();
        let target = version.join("target%literal!name.exe");
        std::fs::write(&target, b"fixture").unwrap();
        let entry = root.join("bin [literal]/sample.cmd");
        let commands = render(&plan(vec![
            Step::SetCurrent {
                program_dir: program.clone(),
                version: "1.0".into(),
            },
            Step::Link {
                path: entry.clone(),
                target: target.clone(),
                register: false,
            },
        ]));
        let powershell = Path::new(&std::env::var("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let output = std::process::Command::new(&powershell)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("$ErrorActionPreference='Stop'; {}", commands.join("; ")),
            ])
            .env("PSModuleAnalysisCachePath", "NUL")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(&entry).unwrap(),
            crate::platform::cmd_shim_text(&target).unwrap()
        );
        assert_eq!(
            crate::platform::read_link(program.join("current")).unwrap(),
            version
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
