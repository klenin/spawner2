[CmdletBinding()]
Param(
    [String]
    $CreateUser,

    [String]
    $Password,

    [Switch]
    $Runner,

    [String[]]
    $Dir,

    [ValidateScript({ Test-Identity -Name $_ })]
    [String]
    $User,

    [Switch]
    $Help
)

$helpMsg = @'
Parameters:
    -CreateUser <username>      Create new user and use it as default.
    -Password <string>          Specify password for a new user.
    -Runner                     Grant following priveleges for default user:
                                    SeAssignPrimaryTokenPrivilege
                                    SeTcbPrivilege
                                    SeIncreaseQuotaPrivilege
                                Allowing it to run processes as another user. 
    -Dir <path>,<path>,...      Grant default user access to these directories.
    -User <username>            Set default user to <username>.
    -Help                       Print this message.

Examples:
    .\setup.ps1 -CreateUser some_user -Password 123 -Dir 'C:\dir1','C:\dir2'
        Creates user 'some_user' with password '123' and grants it access to 'C:\dir1' and 'C:\dir2' directories.

    .\setup.ps1 -User some_user -Dir 'C:\dir1','C:\dir2'
        Grants access to an existing user.
'@

if ($Help) {
    echo "$helpMsg"
}

if (!(Get-Module -ListAvailable -Name 'Carbon')) {
    echo 'Installing Carbon module'
    Install-Module -Name 'Carbon' -AllowClobber
}

Import-Module Carbon

$defaultUser = $env:UserName

if ($CreateUser) {
    if (!($Password)) {
        throw "Password is required."
    }

    $creds = New-Credential -User $CreateUser -Password $Password
    Install-User -Credential $creds
    $defaultUser = $CreateUser
}

if ($User) {
    $defaultUser = $User
}

if ($Runner) {
    $runnerPrivs = @(
        'SeAssignPrimaryTokenPrivilege';
        'SeTcbPrivilege';
        'SeIncreaseQuotaPrivilege';
    )
    foreach ($p in $runnerPrivs) {
        Grant-Privilege -Identity $defaultUser -Privilege $p
    }
}

foreach ($d in $Dir) {
    $permission = [System.Security.AccessControl.FileSystemRights] "Read", "Write", "CreateFiles"
    Grant-Permission -Identity $defaultUser -Permission $permission -Path $d
}
