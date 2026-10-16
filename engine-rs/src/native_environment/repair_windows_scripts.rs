//! Fixed PowerShell reads and setters. User settings never enter the elevated batch.
#[cfg(windows)]
pub(super) const SYSTEM: &str = r#"
function Read-Policy($area,$name) {
 try {
  $p=[Windows.Management.Policies.NamedPolicy,Windows.Management,ContentType=WindowsRuntime]::GetPolicyFromPath($area,$name)
  if ($null -eq $p -or $p.IsUserPolicy) {return $null}
  $value=$null
  if ($p.IsManaged) {switch ($p.Kind.ToString()) {'String' {$value=$p.GetString()} 'Int32' {$value=$p.GetInt32()} default {return $null}}}
  return [ordered]@{managed=[bool]$p.IsManaged;value=$value}
 } catch {return $null}
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
 $r.timezone_policy=Read-Policy 'TimeLanguageSettings' 'ConfigureTimeZone'
 try {$r.zone_start=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\tzautoupdate' -ErrorAction Stop).Start; $r.zone_status=(Get-Service tzautoupdate -ErrorAction Stop).Status.ToString()} catch {}
 $consent=$null
 try {$consent=(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location' -ErrorAction Stop).Value} catch {}
 $r.zone_policy=[ordered]@{consent=$consent;disable_location=(Read-LocationPolicy 'DisableLocation');disable_provider=(Read-LocationPolicy 'DisableWindowsLocationProvider');apps=(Read-Policy 'Privacy' 'LetAppsAccessLocation');master=(Read-Policy 'System' 'AllowLocation')}
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
        let script = r#"
$ErrorActionPreference='Stop'
$type=[Windows.Management.Policies.NamedPolicy,Windows.Management,ContentType=WindowsRuntime]
$result=@{type=$type.FullName;assembly=$type.Assembly.GetName().Name;called=$false;available=$false;hresult=$null}
try {$result.called=$true;$policy=$type::GetPolicyFromPath('System','AllowLocation');$result.available=($null -ne $policy)}
catch {$exception=$_.Exception;while ($exception.InnerException) {$exception=$exception.InnerException};if (@(-2147221164,-2147467263,-2147024846,-2147024891) -notcontains $exception.HResult) {throw};$result.hresult=$exception.HResult}
$result | ConvertTo-Json -Compress
"#;
        let result: serde_json::Value = serde_json::from_str(
            &super::super::windows::script(script)
                .expect("The public WinRT type and documented method must bind"),
        )
        .unwrap();
        assert_eq!(result["type"], "Windows.Management.Policies.NamedPolicy");
        assert_eq!(result["called"], true);
        assert!(result["assembly"]
            .as_str()
            .is_some_and(|name| !name.is_empty()));
        assert!(result["available"] == true || result["hresult"].as_i64().is_some());
        eprintln!("Native policy binding: {result}");
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
