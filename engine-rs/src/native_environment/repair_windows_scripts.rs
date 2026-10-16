//! Fixed PowerShell reads and setters. User settings never enter the elevated batch.
#[cfg(windows)]
pub(super) const SYSTEM: &str = r#"
function Read-Policies {
 $source=@'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$ErrorActionPreference='Stop'
function Read-Policy($area,$name) {
 try {
  $null=[Windows.Management.Policies.NamedPolicy,Windows.Management.Policies,ContentType=WindowsRuntime]
  $p=[Windows.Management.Policies.NamedPolicy]::GetPolicyFromPath($area,$name)
  if ($null -eq $p -or $p.IsUserPolicy) {return $null}
  $value=$null
  if ($p.IsManaged) {switch ($p.Kind.ToString()) {'String' {$value=$p.GetString()} 'Int32' {$value=$p.GetInt32()} default {return $null}}}
  return [ordered]@{managed=[bool]$p.IsManaged;value=$value}
 } catch {throw}
}
try {
 $values=[ordered]@{timezone=(Read-Policy 'TimeLanguageSettings' 'ConfigureTimeZone');apps=(Read-Policy 'Privacy' 'LetAppsAccessLocation');master=(Read-Policy 'System' 'AllowLocation')}
 @{status='read';values=$values} | ConvertTo-Json -Depth 8 -Compress
} catch {$e=$_.Exception;while ($e.InnerException) {$e=$e.InnerException};@{status='error';hresult=$e.HResult;exception=$e.GetType().FullName} | ConvertTo-Json -Compress}
'@
 $start=[Diagnostics.ProcessStartInfo]::new()
 $start.FileName=[IO.Path]::Combine([Environment]::SystemDirectory,'WindowsPowerShell\v1.0\powershell.exe')
 $start.Arguments='-NoLogo -NoProfile -NonInteractive -EncodedCommand '+[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($source))
 $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
 $start.StandardOutputEncoding=[Text.UTF8Encoding]::new($false)
 $start.EnvironmentVariables['PSModuleAnalysisCachePath']='NUL'
 $start.EnvironmentVariables['PSModulePath']=[IO.Path]::Combine([Environment]::SystemDirectory,'WindowsPowerShell\v1.0\Modules')
 $child=[Diagnostics.Process]::new();$child.StartInfo=$start
 try {
  if (-not $child.Start()) {return @{status='unavailable'}}
  $output=$child.StandardOutput.ReadToEndAsync();$errors=$child.StandardError.ReadToEndAsync()
  if (-not $child.WaitForExit(2000)) {$child.Kill();$null=$child.WaitForExit(250);return @{status='timeout'}}
  if (-not $output.Wait(250) -or -not $errors.Wait(250) -or $child.ExitCode -ne 0 -or $output.Result.Length -gt 8192) {return @{status='unavailable'}}
  return ($output.Result | ConvertFrom-Json -ErrorAction Stop)
 } catch {return @{status='unavailable'}} finally {$child.Dispose()}
}
function Update-TimeService {
 Fresh
 & ([IO.Path]::Combine([Environment]::SystemDirectory,'w32tm.exe')) /config /update
 if ($LASTEXITCODE -ne 0) {exit 1}
}
function Read-LocationPolicy($name) {
 $key=$null
 try {
  $key=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SOFTWARE\Policies\Microsoft\Windows\LocationAndSensors')
  if (-not $key -or $key.GetValueNames() -notcontains $name) {return 0}
  $value=$key.GetValue($name)
  if ($value -is [int]) {return $value}
  return $null
 } catch {return $null} finally {if ($key) {$key.Dispose()}}
}
function Read-System {
 $r=[ordered]@{zone=$null; zone_start=$null; zone_status=$null; zone_policy=$null; timezone_policy=$null; ntp=$null; type=$null; policy=$null; ntp_enabled=$null; time_start=$null; service=$null}
 try {$r.zone=(Get-TimeZone -ErrorAction Stop).Id} catch {}
 $policies=Read-Policies
 $r.timezone_policy=$policies.values.timezone
 try {$r.zone_start=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\tzautoupdate' -ErrorAction Stop).Start; $r.zone_status=(Get-Service tzautoupdate -ErrorAction Stop).Status.ToString()} catch {}
 $consent=$null
 try {$consent=(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location' -ErrorAction Stop).Value} catch {}
 $r.zone_policy=[ordered]@{consent=$consent;disable_location=(Read-LocationPolicy 'DisableLocation');disable_provider=(Read-LocationPolicy 'DisableWindowsLocationProvider');apps=$policies.values.apps;master=$policies.values.master}
 try {$p=Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\W32Time\Parameters' -ErrorAction Stop; $r.ntp=$p.NtpServer; $r.type=$p.Type; $r.policy=Test-Path 'HKLM:\SOFTWARE\Policies\Microsoft\W32Time' -ErrorAction Stop; $r.ntp_enabled=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\W32Time\TimeProviders\NtpClient' -ErrorAction Stop).Enabled; $r.time_start=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\W32Time' -ErrorAction Stop).Start; $r.service=(Get-Service W32Time -ErrorAction Stop).Status.ToString()} catch {}
 return $r
}
function Fingerprint($value) { $hash=[Security.Cryptography.SHA256]::Create(); try { return [Convert]::ToBase64String($hash.ComputeHash([Text.Encoding]::UTF8.GetBytes((ConvertTo-Json -InputObject $value -Depth 12 -Compress)))) } finally {$hash.Dispose()} }
"#;

#[cfg(windows)]
pub(super) const USER: &str = r#"
function Read-User {
 $r=[ordered]@{sid=$null; languages=$null; language_supported=$null; culture=$null; independent_formats=$null; formats=$null; variables=$null}
 try {$r.sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value} catch {}
 try {$r.languages=@(Get-WinUserLanguageList -ErrorAction Stop | Select-Object LanguageTag,InputMethodTips,Spellchecking,Handwriting)} catch {}
 try {$r.language_supported=$true; foreach ($tag in $targetTags) {$candidate=New-WinUserLanguageList -Language $tag -ErrorAction Stop; if ($candidate.Count -ne 1 -or $candidate[0].LanguageTag -ine $tag) {$r.language_supported=$false}}} catch {$r.language_supported=$null}
 try {$r.culture=(Get-Culture -ErrorAction Stop).Name; $r.independent_formats=Get-WinCultureFromLanguageListOptOut -ErrorAction Stop} catch {}
 try {
  $key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Control Panel\International')
  if (-not $key) {throw 'Regional formats unavailable'}
  try {$r.formats=[ordered]@{}; foreach ($name in @('iCalendarType','iCountry','iCurrDigits','iCurrency','iDate','iDigits','iFirstDayOfWeek','iFirstWeekOfYear','iLZero','iMeasure','iNegCurr','iNegNumber','iPaperSize','iTime','iTimePrefix','iTLZero','Locale','LocaleName','NumShape','s1159','s2359','sCountry','sCurrency','sDate','sDecimal','sGrouping','sLanguage','sList','sLongDate','sMonDecimalSep','sMonGrouping','sMonThousandSep','sNativeDigits','sNegativeSign','sPositiveSign','sShortDate','sShortTime','sThousand','sTime','sTimeFormat','sYearMonth')) {$r.formats[$name]=$key.GetValue($name,$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)}} finally {$key.Dispose()}
 } catch {$r.formats=$null}
 try {
  $key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')
  try {$r.variables=[ordered]@{}; foreach ($name in $names) {$value=$null; $kind=$null; if ($key -and $key.GetValueNames() -contains $name) {$value=$key.GetValue($name,$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames); $kind=$key.GetValueKind($name).ToString()}; $r.variables[$name]=[ordered]@{value=$value;kind=$kind}}} finally {if ($key) {$key.Dispose()}}
 } catch {$r.variables=$null}
 return $r
}
"#;

pub(super) fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

pub(super) fn user_write(item: &super::repair::Item) -> Option<String> {
    Some(match item.id.as_str() {
        "language.independent_formats" => {
            "Set-WinCultureFromLanguageListOptOut -OptOut $true -ErrorAction Stop".into()
        }
        "language.preferred" => {
            let tags = item
                .expected
                .as_array()?
                .iter()
                .map(|tag| tag.as_str().map(quote))
                .collect::<Option<Vec<_>>>()?;
            let first = tags.first()?;
            format!("if ((Get-WinCultureFromLanguageListOptOut -ErrorAction Stop) -ne $true) {{throw 'Regional format policy changed'}}; $old=Get-WinUserLanguageList -ErrorAction Stop; $next=New-WinUserLanguageList -Language {first} -ErrorAction Stop; $next.Clear(); foreach ($tag in @({})) {{$prior=@($old | Where-Object LanguageTag -ieq $tag); if ($prior.Count -eq 1) {{$next.Add($prior[0])}} else {{$new=New-WinUserLanguageList -Language $tag -ErrorAction Stop; $next.Add($new[0])}}}}; Set-WinUserLanguageList -LanguageList $next -Force -ErrorAction Stop", tags.join(","))
        }
        id if id.strip_prefix("session.").is_some_and(|key| {
            ["TZ", "LANG", "LC_ALL"].contains(&key) || super::LOCALE_CATEGORIES.contains(&key)
        }) =>
        {
            format!(
                "[Environment]::SetEnvironmentVariable({}, {}, 'User')",
                quote(id.strip_prefix("session.")?),
                quote(item.expected.as_str()?)
            )
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn language_item() -> super::super::repair::Item {
        super::super::repair::Item {
            id: "language.preferred".into(),
            scope: "user".into(),
            before: Some(json!(["en-US", "zh-CN"])),
            expected: json!(["zh-CN", "en-US"]),
            status: "change".into(),
        }
    }
    #[test]
    fn literal_values_and_environment_names_are_bounded_to_the_selected_scope() {
        assert_eq!(quote("O'Brien $HOME"), "'O''Brien $HOME'");
        let mut item = language_item();
        assert!(user_write(&item).is_some());
        item.id = "session.USERPROFILE".into();
        item.expected = json!("ignored");
        assert!(user_write(&item).is_none());
        item.id = "session.LANG".into();
        assert!(user_write(&item)
            .unwrap()
            .contains("'LANG', 'ignored', 'User'"));
    }
    #[cfg(windows)]
    #[test]
    fn native_public_policy_binding_executes_without_hiding_type_errors() {
        use std::process::Stdio;
        let marker =
            std::env::temp_dir().join(format!("am-policy-stage-{}", crate::store::uuid_v4()));
        let source = r#"
$ErrorActionPreference='Stop'
$stage='type'
[IO.File]::WriteAllText($marker,$stage)
try {
$type=[Windows.Management.Policies.NamedPolicy,Windows.Management.Policies,ContentType=WindowsRuntime]
$stage='assembly'
[IO.File]::WriteAllText($marker,$stage)
$result=@{type=$type.FullName;assembly=$type.Assembly.GetName().Name;method=$null}
$stage='method'
[IO.File]::WriteAllText($marker,$stage)
$result.method=$type.GetMethod('GetPolicyFromPath').Name
$result | ConvertTo-Json -Compress
} catch {@{stage=$stage;binding_error=$_.Exception.Message;exception_type=$_.Exception.GetType().FullName;hresult=$_.Exception.HResult} | ConvertTo-Json -Compress}
[IO.File]::WriteAllText($marker,'complete')
"#;
        let script = format!(
            "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);$marker={};{source}",
            quote(marker.to_str().unwrap())
        );
        let program = super::super::windows::powershell().unwrap();
        let mut command = crate::process::launch_command(&program);
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ])
            .env("PSModuleAnalysisCachePath", "NUL")
            .env(
                "PSModulePath",
                std::path::Path::new(&program)
                    .parent()
                    .unwrap()
                    .join("Modules"),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output =
            crate::process::capture_group(&mut command, std::time::Duration::from_secs(30), 8192);
        let stage = std::fs::read_to_string(&marker).unwrap_or_else(|_| "not-started".into());
        let _ = std::fs::remove_file(&marker);
        let output =
            output.unwrap_or_else(|error| panic!("Policy probe failed at {stage}: {error}"));
        assert!(
            output.status.success(),
            "Policy probe at {stage}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_str(
            String::from_utf8_lossy(&output.stdout)
                .trim_start_matches('\u{feff}')
                .trim(),
        )
        .unwrap();
        assert!(result["binding_error"].is_null(), "{result}");
        assert_eq!(result["type"], "Windows.Management.Policies.NamedPolicy");
        assert_eq!(result["method"], "GetPolicyFromPath");
        assert!(result["assembly"]
            .as_str()
            .is_some_and(|name| !name.is_empty()));
        eprintln!("Native policy binding: {result}");
        let observed: serde_json::Value = serde_json::from_str(
            &super::super::windows::script_timeout(
                &format!("{SYSTEM} Read-Policies | ConvertTo-Json -Depth 10 -Compress"),
                std::time::Duration::from_secs(15),
            )
            .expect("The bounded native policy reader must finish"),
        )
        .unwrap();
        match observed["status"].as_str() {
            Some("read" | "timeout") => {}
            Some("error") => assert!(
                [-2147221164, -2147467263, -2147024846, -2147024891]
                    .contains(&observed["hresult"].as_i64().unwrap()),
                "Unexpected public policy error: {observed}"
            ),
            other => panic!("Native policy reader failed: {other:?}"),
        }
        eprintln!("Native policy availability: {}", observed["status"]);
    }
    #[cfg(windows)]
    #[test]
    fn blocked_policy_query_keeps_other_system_evidence_and_forbids_policy_assumptions() {
        let start = SYSTEM.find(" $source=@'\n").unwrap() + " $source=@'\n".len();
        let end = start + SYSTEM[start..].find("\n'@").unwrap();
        let mut fixture = SYSTEM.to_string();
        fixture.replace_range(start..end, "Start-Sleep -Seconds 30");
        let result:serde_json::Value=serde_json::from_str(&super::super::windows::script_timeout(
            &format!("{fixture} $r=Read-System; @{{zone_known=($null -ne $r.zone);policy_unknown=($null -eq $r.timezone_policy);location_unknown=($null -eq $r.zone_policy.apps -and $null -eq $r.zone_policy.master)}} | ConvertTo-Json -Compress"),
            std::time::Duration::from_secs(15)
        ).expect("A blocked policy child must not block the system snapshot")).unwrap();
        assert_eq!(result["zone_known"], true);
        assert_eq!(result["policy_unknown"], true);
        assert_eq!(result["location_unknown"], true);
    }
    #[cfg(windows)]
    #[test]
    fn native_language_reordering_retains_objects_and_refuses_missing_format_protection() {
        // All International cmdlets are replaced inside this isolated process;
        // no actual language, registry or user preference setter is called.
        let fixture = r#"
$ErrorActionPreference='Stop'
$script:old=[Collections.Generic.List[object]]::new()
$script:old.Add([pscustomobject]@{LanguageTag='en-US';InputMethodTips=@('custom-a');Spellchecking=$false;Handwriting=$true})
$script:old.Add([pscustomobject]@{LanguageTag='zh-CN';InputMethodTips=@('custom-b');Spellchecking=$true;Handwriting=$false})
$script:protected=$true;$script:next=$null
function Get-WinCultureFromLanguageListOptOut {[CmdletBinding()] param() return $script:protected}
function Get-WinUserLanguageList {[CmdletBinding()] param() return $script:old}
function New-WinUserLanguageList {[CmdletBinding()] param($Language) $list=[Collections.Generic.List[object]]::new();$list.Add([pscustomobject]@{LanguageTag=$Language});return ,$list}
function Set-WinUserLanguageList {[CmdletBinding()] param($LanguageList,[switch]$Force) $script:next=$LanguageList}
"#;
        let setter = user_write(&language_item()).unwrap();
        let script=format!("{fixture} {setter}; if (-not [Object]::ReferenceEquals($next[0],$old[1]) -or -not [Object]::ReferenceEquals($next[1],$old[0])) {{throw 'Existing language options were replaced'}}; $script:next=$null;$script:protected=$false; try {{{setter};throw 'Missing refusal'}} catch {{if ($_.Exception.Message -ne 'Regional format policy changed') {{throw}}}}; if ($null -ne $next) {{throw 'Setter ran without format protection'}}; 'verified'");
        assert_eq!(
            super::super::windows::script(&script).as_deref(),
            Some("verified")
        );
    }
}
