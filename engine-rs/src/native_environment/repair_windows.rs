//! Windows adapters preserve regional formats and keep elevated writes system-only.
use super::{
    repair::{Item, Settings, Snapshot},
    repair_windows_scripts as ps, Target,
};
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub(super) struct WindowsSettings {
    pub persistent: bool,
    target: Target,
    raw: Value,
    formats: Option<Value>,
}
impl WindowsSettings {
    pub fn new(persistent: bool) -> Self {
        Self {
            persistent,
            target: Target::default(),
            raw: Value::Null,
            formats: None,
        }
    }
    fn source(&self) -> String {
        let keys = super::repair::session_keys(&self.target)
            .iter()
            .map(|key| ps::quote(key))
            .collect::<Vec<_>>()
            .join(",");
        let tags = self
            .target
            .languages
            .iter()
            .map(|tag| ps::quote(tag))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{} $names=@({keys}); $targetTags=@({tags}); {}",
            ps::SYSTEM,
            ps::USER
        )
    }
}

impl Settings for WindowsSettings {
    fn snapshot(&mut self, target: &Target) -> Result<Snapshot> {
        self.target = target.clone();
        let source = format!("{} $s=Read-System; $u=Read-User; [ordered]@{{system=$s;user=$u;system_revision=(Fingerprint $s);user_revision=(Fingerprint $u)}} | ConvertTo-Json -Depth 15 -Compress", self.source());
        self.raw = serde_json::from_str(
            &super::windows::script(&source)
                .ok_or_else(|| anyhow::anyhow!("Cannot read Windows repair settings."))?,
        )?;
        let zone = self.raw["system"]["zone"].as_str().and_then(|zone| {
            target
                .timezone
                .as_ref()
                .filter(|target| super::windows::zone_id(target).as_deref() == Some(zone))
                .cloned()
                .or_else(|| super::windows::iana_zone(zone))
        });
        Ok(super::repair_windows_snapshot::decode(
            &self.raw,
            target,
            self.persistent,
            self.formats.as_ref(),
            zone,
            super::windows::geo_name(),
        ))
    }

    fn system(&mut self, items: &[Item]) -> Result<(String, Vec<Item>, Option<bool>)> {
        let started = super::clock_helper::boot_ms()
            .ok_or_else(|| anyhow::anyhow!("Cannot establish an authorization deadline."))?
            % 4_294_967_296;
        let script = system_script(
            items,
            self.raw["system_revision"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing native system baseline."))?,
            self.raw["system"]["ntp"].as_str().unwrap_or(""),
            started,
        )?;
        let code = elevate(&script);
        let snapshot = self.snapshot(&self.target.clone()).ok();
        let mut results = items.to_vec();
        for item in &mut results {
            let actual = snapshot
                .as_ref()
                .filter(|snapshot| !snapshot.unavailable.contains(&item.id))
                .and_then(|snapshot| snapshot.values.get(&item.id).cloned().flatten());
            item.status = if code == Some(126) {
                "not_applied"
            } else if actual.as_ref() == Some(&item.expected) {
                if code == Some(0) {
                    "verified"
                } else {
                    "verified_after_error"
                }
            } else if actual.is_none() {
                "outcome_unknown"
            } else if code == Some(73) {
                "stale"
            } else {
                "unverified"
            }
            .into();
        }
        let status = if code == Some(126) {
            "cancelled"
        } else if code == Some(0) && results.iter().all(|item| item.status == "verified") {
            "applied"
        } else {
            "partial"
        };
        Ok((status.into(), results, None))
    }

    fn write_user(&mut self, item: &Item) -> bool {
        let Some(revision) = self.raw["user_revision"].as_str() else {
            return false;
        };
        let guard=format!("{} if ((Fingerprint (Read-User)) -cne {}) {{throw 'User settings changed after preview'}}; ",self.source(),ps::quote(revision));
        if item.id == "language.region" {
            return super::windows::script(&guard).is_some()
                && super::windows::geo_name()
                    .as_ref()
                    .map(|value| json!(value))
                    == item.before
                && item
                    .expected
                    .as_str()
                    .is_some_and(super::windows::set_geo_name);
        }
        if item.id == "language.preferred" {
            self.formats = Some(
                json!({"culture":self.raw["user"]["culture"],"formats":self.raw["user"]["formats"]}),
            );
        }
        let Some(write) = ps::user_write(item) else {
            return false;
        };
        super::windows::script(&format!("$ErrorActionPreference='Stop'; {guard}{write}")).is_some()
    }
}

// Only fixed system setters reach this boundary. HKCU and current-user APIs
// always execute in the original caller, even when UAC uses another administrator.
fn elevate(script: &str) -> Option<i32> {
    let program = super::windows::powershell()?;
    let command=format!("$encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes({})); try {{$child=Start-Process -FilePath {} -Verb RunAs -Wait -PassThru -ArgumentList ('-NoLogo -NoProfile -NonInteractive -EncodedCommand '+$encoded) -ErrorAction Stop; $child.ExitCode}} catch {{if ($_.Exception.NativeErrorCode -eq 1223 -or $_.Exception.InnerException.NativeErrorCode -eq 1223) {{126}} else {{1}}}}",ps::quote(script),ps::quote(&program));
    if command.len() > 24_000 {
        return None;
    }
    super::windows::script_timeout(&command, std::time::Duration::from_secs(150))?
        .parse()
        .ok()
}

fn system_script(items: &[Item], baseline: &str, ntp: &str, started: u64) -> Result<String> {
    let fresh=format!("function Fresh {{$now=[int64][Environment]::TickCount; if ($now -lt 0) {{$now+=4294967296}}; if ((($now-{started}+4294967296)%4294967296) -ge 120000) {{exit 74}}}}; Fresh;");
    let mut script = format!("$ErrorActionPreference='Stop'; $env:PSModuleAnalysisCachePath='NUL'; $env:PSModulePath=[IO.Path]::Combine($PSHOME,'Modules'); {fresh} {} $baseline={}; if ((Fingerprint (Read-System)) -cne $baseline) {{exit 73}}; try {{", ps::SYSTEM, ps::quote(baseline));
    for item in items {
        let (write, verify) = match item.id.as_str() {
            "time.timezone" => {
                let zone = super::windows::zone_id(item.expected.as_str().unwrap_or(""))
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "This Windows version cannot map the selected IANA time zone."
                        )
                    })?;
                (
                    format!("Set-TimeZone -Id {} -ErrorAction Stop", ps::quote(&zone)),
                    format!("$after.zone -ieq {}", ps::quote(&zone)),
                )
            }
            "time.automatic_timezone" => {
                if item.expected == true {
                    ("Fresh; Set-Service tzautoupdate -StartupType Manual -ErrorAction Stop; Fresh; Start-Service tzautoupdate -ErrorAction Stop".into(), "$after.zone_start -eq 3".into())
                } else {
                    ("Fresh; Stop-Service tzautoupdate -ErrorAction Stop; Fresh; Set-Service tzautoupdate -StartupType Disabled -ErrorAction Stop".into(), "$after.zone_start -eq 4".into())
                }
            }
            "ntp.server" => {
                let peer = super::repair_windows_snapshot::peer_with_host(
                    ntp,
                    item.expected.as_str().unwrap_or(""),
                )
                .ok_or_else(|| anyhow::anyhow!("Cannot preserve the current NTP peer flags."))?;
                (format!("Fresh; Set-ItemProperty 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\W32Time\\Parameters' -Name NtpServer -Value {} -ErrorAction Stop; if ((Get-Service W32Time).Status -eq 'Running') {{Update-TimeService}}",ps::quote(&peer)),format!("$after.ntp -ceq {}",ps::quote(&peer)))
            }
            "ntp.automatic_time" => {
                if item.expected == true {
                    ("Fresh; Set-ItemProperty 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\W32Time\\Parameters' -Name Type -Value 'NTP' -ErrorAction Stop; Fresh; Set-ItemProperty 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\W32Time\\TimeProviders\\NtpClient' -Name Enabled -Value 1 -ErrorAction Stop; Fresh; Set-Service W32Time -StartupType Automatic -ErrorAction Stop; Fresh; Start-Service W32Time -ErrorAction Stop; Update-TimeService".into(), "$after.type -eq 'NTP' -and $after.ntp_enabled -eq 1 -and $after.time_start -eq 2".into())
                } else {
                    ("Fresh; Set-ItemProperty 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\W32Time\\Parameters' -Name Type -Value 'NoSync' -ErrorAction Stop; if ((Get-Service W32Time).Status -eq 'Running') {Update-TimeService}".into(), "$after.type -eq 'NoSync'".into())
                }
            }
            _ => bail!("Unsupported native system repair item."),
        };
        script.push_str(&format!("Fresh; if ((Fingerprint (Read-System)) -cne $baseline) {{exit 73}}; Fresh; {write}; $after=Read-System; if (-not ({verify})) {{exit 75}}; $baseline=Fingerprint $after; "));
    }
    script.push_str("exit 0 } catch {exit 1}");
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_system_batch_preserves_peer_flags_and_rejects_stale_or_expired_requests() {
        // Native PowerShell executes the production batch, with all OS setters
        // and service updates replaced by functions that write only a fixture log.
        let root =
            std::env::temp_dir().join(format!("am-system-batch-{}", crate::store::uuid_v4()));
        std::fs::create_dir(&root).unwrap();
        let items = [
            ("time.timezone", json!("Etc/UTC")),
            ("ntp.server", json!("new.example")),
            ("ntp.automatic_time", json!(true)),
        ]
        .into_iter()
        .map(|(id, expected)| Item {
            id: id.into(),
            scope: "system".into(),
            before: None,
            expected,
            status: "change".into(),
        })
        .collect::<Vec<_>>();
        let tick = super::super::clock_helper::boot_ms().unwrap() % 4_294_967_296;
        for (case, baseline, started) in [
            ("applied", "initial", tick),
            ("stale", "changed", tick),
            (
                "expired",
                "initial",
                (tick + 4_294_967_296 - 120001) % 4_294_967_296,
            ),
        ] {
            let trace = root.join(case);
            let fixture = format!(
                r#"
$script:trace={}
$script:state=@{{zone='Old';ntp='old.example,0x9';type='NoSync';ntp_enabled=1;time_start=3;service='Stopped';revision='initial'}}
function Read-System {{return $script:state}}
function Fingerprint($value) {{return $value.revision}}
function Record($value) {{$script:state.revision+='x';[IO.File]::AppendAllText($script:trace,$value+"`n")}}
function Set-TimeZone {{[CmdletBinding()] param($Id) $script:state.zone=$Id;Record ('zone='+$Id)}}
function Set-ItemProperty {{[CmdletBinding()] param($Path,$Name,$Value) switch ($Name) {{'NtpServer' {{$script:state.ntp=$Value}} 'Type' {{$script:state.type=$Value}} 'Enabled' {{$script:state.ntp_enabled=$Value}} default {{throw 'Unexpected property'}}}};Record ($Name+'='+$Value)}}
function Get-Service {{[CmdletBinding()] param($Name) [pscustomobject]@{{Status=$script:state.service}}}}
function Set-Service {{[CmdletBinding()] param($Name,$StartupType) if ($Name -ne 'W32Time' -or $StartupType -ne 'Automatic') {{throw 'Unexpected service change'}};$script:state.time_start=2;Record 'startup=Automatic'}}
function Start-Service {{[CmdletBinding()] param($Name) if ($Name -ne 'W32Time') {{throw 'Unexpected service'}};$script:state.service='Running';Record 'service=Running'}}
function Update-TimeService {{Fresh;Record 'update'}}
"#,
                ps::quote(trace.to_str().unwrap())
            );
            let script = system_script(&items, baseline, "old.example,0x9", started).unwrap();
            assert!(script.contains(ps::SYSTEM));
            let result = super::super::windows::script(&script.replace(ps::SYSTEM, &fixture));
            if case == "applied" {
                assert_eq!(result.as_deref(), Some(""));
                assert_eq!(std::fs::read_to_string(&trace).unwrap(),"zone=UTC\nNtpServer=new.example,0x9\nType=NTP\nEnabled=1\nstartup=Automatic\nservice=Running\nupdate\n");
            } else {
                assert!(result.is_none(), "{case}");
                assert!(!trace.exists(), "No setter may run for {case}");
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
