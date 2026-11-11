function Get-DispatchPairLeaderReviewPrompt {
    param(
        [int]$AttemptedPairs,
        [int]$AvailablePairs,
        [int]$P95RegressionPairs,
        [int]$P99RegressionPairs
    )

    if ($AttemptedPairs -ne 6 -or $AvailablePairs -ne 6) {
        return $null
    }

    $P95RegressionPairs -ge 4 -or $P99RegressionPairs -ge 4
}

Export-ModuleMember -Function Get-DispatchPairLeaderReviewPrompt
