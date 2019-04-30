Import-Module Carbon

echo '1. Configure privileges...'

$user = Read-Host -Prompt 'Input the user name (press enter to select current user)'
if ($user -eq "") {
    $user = $env:UserName
}

if (!(Test-Identity -Name $user)) {
    echo "Bad user name: '$user'"
    exit
}

$privileges = @(
    'SeAssignPrimaryTokenPrivilege';
    'SeTcbPrivilege';
    'SeIncreaseQuotaPrivilege';
)

echo "The following privileges will be enabled for '$user' user:"
$privileges | % { echo "`t$_" }

$answer = ''
while (($answer -ne "y") -and ($answer -ne "n")) {
    $answer = Read-Host -Prompt 'Enable those privileges? [y/n]'
}

if ($answer -eq "y") {
    $privileges | % { Grant-Privilege -Identity $user -Privilege $_ }
    echo "Windows restart is needed to apply changes"
}
