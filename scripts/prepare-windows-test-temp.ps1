$ErrorActionPreference = 'Stop'
$currentUser = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$trustedSids = @($currentUser.Value, 'S-1-5-18', 'S-1-5-32-544')
$localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
if (-not [IO.Path]::IsPathFullyQualified($localAppData)) { throw 'Missing OS LocalAppData directory' }

# Report permission categories only; account paths, names and credentials are absent.
foreach ($source in @(@{ Phase = 'runtime'; Path = $localAppData }, @{ Phase = 'original-temp'; Path = [IO.Path]::GetTempPath() })) {
    $ancestors = [Collections.Generic.List[IO.DirectoryInfo]]::new()
    $directory = [IO.DirectoryInfo]::new($source.Path)
    while ($null -ne $directory) { $ancestors.Add($directory); $directory = $directory.Parent }
    $ancestors.Reverse()
    for ($depth = 0; $depth -lt $ancestors.Count; $depth++) {
        $acl = Get-Acl -LiteralPath $ancestors[$depth].FullName
        $owner = $acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value
        $rules = @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]) | ForEach-Object {
            @{ trusted = $trustedSids -contains $_.IdentityReference.Value; type = "$($_.AccessControlType)"; mask = ([long]$_.FileSystemRights -band 0xffffffff); inheritOnly = ($_.PropagationFlags -band [System.Security.AccessControl.PropagationFlags]::InheritOnly) -ne 0 }
        })
        @{ phase = $source.Phase; depth = $depth; root = $depth -eq 0; trustedOwner = $trustedSids -contains $owner; rules = $rules } | ConvertTo-Json -Compress -Depth 4 | Write-Output
    }
}

# Create only a fresh CI fixture directory, with its private ACL at creation.
# https://learn.microsoft.com/dotnet/api/system.io.filesystemaclextensions.create
$testRoot = Join-Path $localAppData ('AgentManager-CI-' + [Guid]::NewGuid().ToString('N'))
if (Test-Path -LiteralPath $testRoot) { throw 'CI fixture directory already exists' }
$security = [System.Security.AccessControl.DirectorySecurity]::new()
$security.SetOwner($currentUser)
$security.SetAccessRuleProtection($true, $false)
$inheritance = [System.Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [System.Security.AccessControl.InheritanceFlags]::ObjectInherit
$security.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($currentUser, [System.Security.AccessControl.FileSystemRights]::FullControl, $inheritance, [System.Security.AccessControl.PropagationFlags]::None, [System.Security.AccessControl.AccessControlType]::Allow))
[IO.FileSystemAclExtensions]::Create([IO.DirectoryInfo]::new($testRoot), $security)
$createdAcl = Get-Acl -LiteralPath $testRoot
if (-not $createdAcl.AreAccessRulesProtected -or $createdAcl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value -ne $currentUser.Value) { throw 'CI fixture ACL did not retain its private owner and protection' }
foreach ($rule in $createdAcl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier])) {
    if ($rule.AccessControlType -eq 'Allow' -and $rule.IdentityReference.Value -ne $currentUser.Value) { throw 'CI fixture ACL grants another identity access' }
}
"TEMP=$testRoot" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
"TMP=$testRoot" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
Write-Output 'Prepared a fresh private Windows test directory under the OS user profile.'
