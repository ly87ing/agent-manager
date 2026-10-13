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

pub(super) fn build(spec: &Recipe, variant: &Variant) -> Built {
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
        format!("& {{ $p = New-Object System.Diagnostics.ProcessStartInfo; $p.FileName = {}; $p.UseShellExecute = $false; $p.Arguments = {}; $p.EnvironmentVariables['TZ'] = {}; {completion} }}",
            powershell(&spec.target), powershell(&args), powershell(&variant.timezone))
    } else if spec.platform == "macos" && spec.kind == "app" {
        let mut value = format!(
            "open --env {} -a {}",
            posix(&format!("TZ={}", variant.timezone)),
            posix(&spec.target)
        );
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
        format!(
            "TZ={} command {}{}",
            posix(&variant.timezone),
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
        timezone: variant.timezone.clone(),
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
        let definition = build(
            &Recipe {
                platform: "windows".into(),
                shell: "powershell".into(),
                kind: "app".into(),
                target: r"C:\Apps\Example.exe".into(),
                args: vec![],
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
