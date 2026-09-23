$ErrorActionPreference = 'Stop'
$value = 0.123456789
while ($true) {
    $value = [Math]::Sqrt(($value + 12345.6789) * 987.6543)
}
