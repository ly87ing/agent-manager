use super::{Recipe, Variant};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct Built {
    pub name: String,
    pub timezone: String,
    pub command: String,
    pub definition: String,
}

/// Quote literal arguments twice when embedding a complete command in an alias.
pub(super) fn posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn powershell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Windows native executable argument quoting (backslashes before quotes/end).
fn windows_arg(value: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        out.push_str(&"\\".repeat(if c == '"' { slashes * 2 + 1 } else { slashes }));
        out.push(c);
        slashes = 0;
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

// Batch files double embedded quotes; the empty CD expansion protects literal %.
fn batch_arg(value: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        out.push_str(&"\\".repeat(if c == '"' { slashes * 2 } else { slashes }));
        slashes = 0;
        match c {
            '"' => out.push_str("\"\""),
            '%' => out.push_str("%%cd:~,%"),
            _ => out.push(c),
        }
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

pub(super) fn build(spec: &Recipe, variant: &Variant) -> Built {
    let environment = spec.environment(variant);
    let timezone = environment
        .iter()
        .find(|(name, _)| {
            if spec.platform == "windows" {
                name.eq_ignore_ascii_case("TZ")
            } else {
                *name == "TZ"
            }
        })
        .map(|(_, value)| *value)
        .unwrap_or_default()
        .to_owned();
    let command = if spec.platform == "windows" {
        let args = spec
            .args
            .iter()
            .map(|v| windows_arg(v))
            .collect::<Vec<_>>()
            .join(" ");
        let completion = if spec.kind == "command" {
            "$c = [System.Diagnostics.Process]::Start($p); $c.WaitForExit(); $global:LASTEXITCODE = $c.ExitCode"
        } else {
            "[System.Diagnostics.Process]::Start($p) | Out-Null"
        };
        let environment = environment
            .iter()
            .map(|(name, value)| {
                format!(
                    "$p.EnvironmentVariables[{}] = {};",
                    powershell(name),
                    powershell(value)
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        let resolve = if spec.kind == "command" {
            r#"if ([IO.Path]::GetExtension($p.FileName.Trim().Trim('"')) -eq '' -and -not [IO.File]::Exists($p.FileName + '.exe')) { $batch = Get-Command -Name ([System.Management.Automation.WildcardPattern]::Escape($p.FileName)) -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1; if ($batch -and [IO.Path]::GetExtension($batch.Source) -in @('.cmd', '.bat')) { $p.FileName = $batch.Source; } }"#
        } else {
            ""
        };
        let batch_args = spec
            .args
            .iter()
            .map(|arg| batch_arg(arg))
            .collect::<Vec<_>>()
            .join(" ");
        let batch = format!(
            r#"$batchPath = $p.FileName.Trim().Trim('"').TrimEnd([char[]]' .'); if ($batchPath -match '\.(cmd|bat)$') {{ if ($batchPath -ne $p.FileName -or $batchPath.Contains('"')) {{ throw 'Batch paths must not use quotes or surrounding spaces/dots' }} $p.Arguments = '/e:ON /v:OFF /d /c ""' + $p.FileName.Replace('%', '%%cd:~,%') + '" ' + {} + '"'; $p.FileName = [IO.Path]::Combine([Environment]::SystemDirectory, 'cmd.exe'); if ($p.FileName.Length + $p.Arguments.Length + 3 -gt 8191) {{ throw 'Batch command exceeds the Windows command-line limit' }} }}"#,
            powershell(&batch_args)
        );
        format!("& {{ $p = New-Object System.Diagnostics.ProcessStartInfo; $p.FileName = {}; $p.UseShellExecute = $false; $p.Arguments = {}; {resolve} {batch} {environment} {completion} }}",
            powershell(&spec.target), powershell(&args))
    } else if spec.platform == "macos" && spec.kind == "app" {
        let mut value = String::from("open");
        for (name, environment_value) in environment {
            value.push_str(" --env ");
            value.push_str(&posix(&format!("{name}={environment_value}")));
        }
        value.push_str(" -a ");
        value.push_str(&posix(&spec.target));
        if !spec.args.is_empty() {
            value.push_str(" --args ");
            value.push_str(
                &spec
                    .args
                    .iter()
                    .map(|v| posix(v))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        value
    } else {
        let environment = environment
            .iter()
            .map(|(name, value)| format!("{name}={}", posix(value)))
            .collect::<Vec<_>>();
        let prefix = if environment.is_empty() {
            String::new()
        } else {
            format!("{} ", environment.join(" "))
        };
        format!(
            "{prefix}command {}{}",
            posix(&spec.target),
            spec.args
                .iter()
                .map(|v| format!(" {}", posix(v)))
                .collect::<String>()
        )
    };
    let definition = if spec.platform == "windows" {
        // A named function can be pasted into $PROFILE or the current session.
        format!("function {} {{ {} }}", variant.name, command)
    } else {
        format!("alias {}={}", variant.name, posix(&command))
    };
    Built {
        name: variant.name.clone(),
        timezone,
        command,
        definition,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_quotes_are_literal_and_windows_backslashes_survive() {
        assert_eq!(posix("O'Brien $HOME"), "'O'\\''Brien $HOME'");
        assert_eq!(windows_arg("a\\\"b\\"), "\"a\\\\\\\"b\\\\\"");
        assert_eq!(powershell("O'Brien"), "'O''Brien'");
        assert_eq!(batch_arg(""), "\"\"");
        assert_eq!(batch_arg("a\\\"b\\"), "\"a\\\\\"\"b\\\\\"");
        assert_eq!(
            batch_arg("%PATH% ! & | < > ^"),
            "\"%%cd:~,%PATH%%cd:~,% ! & | < > ^\""
        );
        let definition = build(
            &Recipe {
                platform: "windows".into(),
                shell: "powershell".into(),
                kind: "app".into(),
                target: r"C:\Apps\Example.exe".into(),
                args: vec![],
                env: vec![],
                variants: vec![Variant {
                    name: "example-sg".into(),
                    timezone: "Asia/Singapore".into(),
                }],
            },
            &Variant {
                name: "example-sg".into(),
                timezone: "Asia/Singapore".into(),
            },
        )
        .definition;
        assert!(definition.starts_with("function example-sg { "));
        assert!(definition.ends_with(" }"));
    }
}
