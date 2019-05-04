[CmdletBinding()]
Param(
    [String]
    $CreateUser,

    [String]
    $Password,

    [ValidateScript({ Test-Identity -Name $_ })]
    [String]
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
    -CreateUser <username>      Creates a new user and uses it as current.
    -Password <string>          Specifies password for a new user.
    -Runner <username>          Grants the following priveleges for <username>:
                                    SeAssignPrimaryTokenPrivilege
                                    SeTcbPrivilege
                                    SeIncreaseQuotaPrivilege
                                Allowing it to run processes as another user. 
    -Dir <path>,<path>,...      Grants current user access to these directories.
    -User <username>            Sets current user to <username>.
    -Help                       Prints this message.

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

if ($Runner) {
    $runnerPrivs = @(
        'SeAssignPrimaryTokenPrivilege';
        'SeTcbPrivilege';
        'SeIncreaseQuotaPrivilege';
    )
    foreach ($p in $runnerPrivs) {
        Grant-Privilege -Identity $Runner -Privilege $p
    }
}

$currentUser = ''

if ($CreateUser) {
    if (!($Password)) {
        throw "Password is required."
    }

    $creds = New-Credential -User $CreateUser -Password $Password
    Install-User -Credential $creds
    $currentUser = $CreateUser
}

if ($User) {
    $currentUser = $User
}

if (!($currentUser) -and $Dir) {
    throw "Please specify a user."
}

foreach ($d in $Dir) {
    Grant-Permission -Identity $currentUser -Permission FullControl -Path $d
}
