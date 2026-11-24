Set-StrictMode -Version Latest

function ConvertTo-DispatchQualificationCollection {
    [CmdletBinding()]
    param([AllowNull()] [object]$Value)

    [object[]]$items = @()
    if ($null -ne $Value) {
        if ($Value -is [System.Array]) {
            $items = [object[]]$Value
        } else {
            $items = [object[]]@($Value)
        }
    }

    [pscustomobject]@{
        Items = $items
        Length = $items.Length
    }
}

function Get-DispatchQualificationProperty {
    param([AllowNull()] [object]$Object, [Parameter(Mandatory)] [string]$Name)

    if ($null -eq $Object) { return $null }
    if ($Object -is [System.Collections.IDictionary]) {
        if ($Object.Contains($Name)) { return $Object[$Name] }
        return $null
    }

    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    $property.Value
}

function New-DispatchSessionRedesignM3FinalPlan {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [object]$Profile,
        [Parameter(Mandatory)] [int]$Pairs,
        [Parameter(Mandatory)] [int]$TimingMarginUs,
        [Parameter(Mandatory)] [int]$SampleSeconds
    )

    if ($Profile.Name -ne 'DispatchSessionRedesignM3') {
        throw 'M3 Final orchestration requires the DispatchSessionRedesignM3 profile'
    }

    $canaryCollection = ConvertTo-DispatchQualificationCollection $Profile.CanaryCase
    $safetyCollection = ConvertTo-DispatchQualificationCollection $Profile.SafetyCases
    $workloadCollection = ConvertTo-DispatchQualificationCollection $Profile.PerformanceCase.Workload
    $loadCollection = ConvertTo-DispatchQualificationCollection $Profile.PerformanceCase.LoadCells

    [object[]]$canaryCases = $canaryCollection.Items
    [object[]]$safetyCases = $safetyCollection.Items
    [string[]]$performanceWorkloads = $workloadCollection.Items
    [string[]]$loadCells = $loadCollection.Items

    $performanceCase = $Profile.PerformanceCase
    if ($canaryCases.Length -ne 1 -or $safetyCases.Length -ne 7 -or
        $performanceWorkloads.Length -ne 1 -or $performanceWorkloads[0] -ne 'mixed-up-down' -or
        ($loadCells -join '|') -ne 'Quiet|Contention' -or
        $Pairs -ne 6 -or $Pairs -ne $performanceCase.PairCount -or
        $TimingMarginUs -ne 500 -or $SampleSeconds -ne 15 -or
        $performanceCase.TimingMarginUs -ne 500 -or $performanceCase.GameFps -ne 60 -or
        $performanceCase.SampleSeconds -ne 15) {
        throw 'DispatchSessionRedesignM3 Final plan must retain one canary, seven safety cases, one mixed-up-down workload, two load cells, six pairs, and approved timing settings'
    }

    if ($canaryCases[0].Scenario -ne 'canonical-single' -or
        $canaryCases[0].Workload -ne 'canonical-single' -or
        $canaryCases[0].Load -ne 'Quiet' -or
        $canaryCases[0].GameFps -ne 60 -or
        $canaryCases[0].TimingMarginUs -ne 500 -or
        $canaryCases[0].SampleSeconds -ne 0) {
        throw 'DispatchSessionRedesignM3 canary identity differs from the approved canonical-single case'
    }

    [string[]]$safetyIds = @($safetyCases | ForEach-Object { [string]$_.Id })
    if (($safetyIds -join '|') -ne (@($Profile.SafetyScenarios) -join '|')) {
        throw 'DispatchSessionRedesignM3 safety case order differs from the profile scenario order'
    }
    foreach ($case in $safetyCases) {
        if ($case.Load -ne 'Quiet' -or $case.TimingMarginUs -ne 500 -or $case.SampleSeconds -ne 0 -or
            ($case.Scenario -eq 'near-minimum-retrigger' -and $case.Workload -ne 'near-minimum-retrigger') -or
            ($case.Scenario -ne 'near-minimum-retrigger' -and $case.Workload -ne 'safety-smoke')) {
            throw ('DispatchSessionRedesignM3 safety case identity is invalid: ' + $case.Id)
        }
    }

    [System.Collections.Generic.List[object]]$plannedCases = [System.Collections.Generic.List[object]]::new()
    $canary = $canaryCases[0]
    $plannedCases.Add([pscustomobject]@{
        phase = 'canary'
        id = [string]$canary.Id
        scenario = [string]$canary.Scenario
        workload = [string]$canary.Workload
        role = 'candidate'
        load = [string]$canary.Load
        cell_id = 'm3/canary/' + $canary.Id + '/' + $canary.Load + '/' + $canary.GameFps + 'fps'
        pair_id = ''
        pair_order = ''
        margin = [int]$canary.TimingMarginUs
        fps = [int]$canary.GameFps
        sample = [int]$canary.SampleSeconds
        historical = $false
    })

    foreach ($case in $safetyCases) {
        $plannedCases.Add([pscustomobject]@{
            phase = 'safety'
            id = [string]$case.Id
            scenario = [string]$case.Scenario
            workload = [string]$case.Workload
            role = 'candidate'
            load = [string]$case.Load
            cell_id = 'm3/safety/' + $case.Id + '/' + $case.Load + '/' + $case.GameFps + 'fps'
            pair_id = ''
            pair_order = ''
            margin = [int]$case.TimingMarginUs
            fps = [int]$case.GameFps
            sample = [int]$case.SampleSeconds
            historical = $false
        })
    }

    foreach ($workload in $performanceWorkloads) {
        foreach ($load in $loadCells) {
            $cellId = $workload + '/' + $load
            for ($pair = 1; $pair -le $Pairs; $pair++) {
                $pairId = $cellId + '/' + ('{0:D2}' -f $pair)
                $pairOrder = if ($pair % 2 -eq 1) { 'AB' } else { 'BA' }
                [string[]]$roles = if ($pairOrder -eq 'AB') { @('control', 'candidate') } else { @('candidate', 'control') }
                foreach ($role in $roles) {
                    $plannedCases.Add([pscustomobject]@{
                        phase = 'performance'
                        id = $pairId + '/' + $role
                        scenario = [string]$performanceCase.Scenario
                        workload = [string]$workload
                        role = [string]$role
                        load = [string]$load
                        cell_id = $cellId
                        pair_id = $pairId
                        pair_order = $pairOrder
                        margin = [int]$TimingMarginUs
                        fps = [int]$performanceCase.GameFps
                        sample = [int]$SampleSeconds
                        historical = $role -eq 'control'
                    })
                }
            }
        }
    }

    [object[]]$caseArray = $plannedCases.ToArray()
    $candidateCount = 0
    $controlCount = 0
    foreach ($case in $caseArray) {
        if ($case.role -eq 'candidate') { $candidateCount++ }
        elseif ($case.role -eq 'control') { $controlCount++ }
        else { throw ('Unsupported M3 Final role: ' + $case.role) }
    }

    if ($caseArray.Length -ne 32 -or $candidateCount -ne 20 -or $controlCount -ne 12) {
        throw 'DispatchSessionRedesignM3 Final plan must contain 32 cases: 20 Candidate and 12 Baseline'
    }

    [pscustomobject]@{
        Stage = 'Final'
        Profile = [string]$Profile.Name
        CanaryCase = $canaryCases[0]
        SafetyCases = $safetyCases
        SafetyScenarios = $safetyIds
        PerformanceWorkloads = $performanceWorkloads
        PerformanceLoadCells = $loadCells
        PairCount = [int]$Pairs
        TimingMarginUs = [int]$TimingMarginUs
        SampleSeconds = [int]$SampleSeconds
        Cases = $caseArray
        PlannedSessionCount = $caseArray.Length
        CandidateSessionCount = $candidateCount
        ControlSessionCount = $controlCount
    }
}

function Invoke-DispatchSessionRedesignM3FinalPlan {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [object]$Plan,
        [Parameter(Mandatory)] [scriptblock]$InvokeCase
    )

    if ($Plan.Profile -ne 'DispatchSessionRedesignM3' -or $Plan.Stage -ne 'Final') {
        throw 'M3 Final dispatcher received a plan for a different profile or stage'
    }

    [System.Collections.Generic.List[object]]$attemptedCases = [System.Collections.Generic.List[object]]::new()
    [System.Collections.Generic.List[object]]$attemptedRows = [System.Collections.Generic.List[object]]::new()
    $stoppedAtFirstNonPass = $false
    foreach ($case in $Plan.Cases) {
        $row = & $InvokeCase $case
        if ($null -eq $row) { throw 'M3 Final case callback did not return a completed row' }
        $attemptedCases.Add($case)
        $attemptedRows.Add($row)
        $caseExitCode = Get-DispatchQualificationProperty $row 'case_exit_code'
        $status = Get-DispatchQualificationProperty $row 'status'
        if ($null -eq $caseExitCode -or [int]$caseExitCode -ne 0 -or [string]$status -ne 'PASS') {
            $stoppedAtFirstNonPass = $true
            break
        }
    }

    [pscustomobject]@{
        Plan = $Plan
        AttemptedCases = $attemptedCases.ToArray()
        AttemptedRows = $attemptedRows.ToArray()
        AttemptedSessionCount = $attemptedRows.Count
        StoppedAtFirstNonPass = $stoppedAtFirstNonPass
    }
}

function Invoke-DispatchSessionRedesignM3FinalControlPath {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [object]$Profile,
        [Parameter(Mandatory)] [int]$Pairs,
        [Parameter(Mandatory)] [int]$TimingMarginUs,
        [Parameter(Mandatory)] [int]$SampleSeconds,
        [AllowNull()] [scriptblock]$OnPlan,
        [Parameter(Mandatory)] [scriptblock]$InvokeCase
    )

    # Production Final and the offline contract both enter through this plan,
    # then differ only in the injected case callback.
    $plan = New-DispatchSessionRedesignM3FinalPlan -Profile $Profile -Pairs $Pairs `
        -TimingMarginUs $TimingMarginUs -SampleSeconds $SampleSeconds
    if ($null -ne $OnPlan) { & $OnPlan $plan | Out-Host }
    $execution = Invoke-DispatchSessionRedesignM3FinalPlan -Plan $plan -InvokeCase $InvokeCase
    [pscustomobject]@{
        Plan = $plan
        Execution = $execution
    }
}

function New-QualificationRunnerFailureArtifacts {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [System.Management.Automation.ErrorRecord]$ErrorRecord,
        [Parameter(Mandatory)] [string]$RunId,
        [Parameter(Mandatory)] [string]$Stage,
        [AllowNull()] [string]$SourceRevision,
        [AllowNull()] [string]$SourceTreeSha,
        [Parameter(Mandatory)] [int]$AttemptedSessionCount,
        [Parameter(Mandatory)] [string]$EvidenceRoot,
        [AllowNull()] [object[]]$ExistingFailures = @(),
        [AllowNull()] [string]$FallbackScriptPath,
        [ValidateSet(1, 2)] [int]$SummarySchemaVersion = 1
    )

    $invocation = $ErrorRecord.InvocationInfo
    $scriptPath = if ($null -ne $invocation) { [string]$invocation.ScriptName } else { '' }
    if ([string]::IsNullOrWhiteSpace($scriptPath)) { $scriptPath = [string]$FallbackScriptPath }
    $lineNumber = 0
    if ($null -ne $invocation) { $lineNumber = [int]$invocation.ScriptLineNumber }
    $line = if ($null -ne $invocation) { [string]$invocation.Line } else { '' }
    $reason = [string]$ErrorRecord.Exception.Message
    $failure = [pscustomobject]@{
        scenario = 'runner'
        workload = $null
        role = $null
        cell_id = $null
        pair_id = $null
        exit_code = 2
        status = 'RUNNER_FAILURE'
        reason = $reason
        evidence_directory = $EvidenceRoot
    }

    [object[]]$failureRows = @($ExistingFailures) + @($failure)
    $summary = [ordered]@{
        schema_version = $SummarySchemaVersion
        run_id = $RunId
        stage = $Stage
        source_revision = $SourceRevision
        source_tree_sha = $SourceTreeSha
        run_count = [int]$AttemptedSessionCount
        failures = $failureRows
        disposition = 'runner failed before completing protocol'
    }
    $errorDetail = [ordered]@{
        schema_version = 1
        message = $reason
        exception_type = $ErrorRecord.Exception.GetType().FullName
        exception_to_string = $ErrorRecord.Exception.ToString()
        script_path = $scriptPath
        line_number = $lineNumber
        line = $line
        script_stack_trace = [string]$ErrorRecord.ScriptStackTrace
        fully_qualified_error_id = [string]$ErrorRecord.FullyQualifiedErrorId
        category = [string]$ErrorRecord.CategoryInfo
    }
    $runDisposition = if ($AttemptedSessionCount -eq 0) { 'NotRun' } else { 'runner failed after one or more case attempts' }
    $summaryMarkdown = @(
        '# Qualification runner failure',
        '',
        ('- Run: `' + $RunId + '`'),
        ('- Stage: `' + $Stage + '`'),
        ('- Source: `' + $SourceRevision + '` tree `' + $SourceTreeSha + '`'),
        ('- Native cases attempted: ' + $AttemptedSessionCount + '; disposition: `' + $runDisposition + '`.'),
        ('- Runner error: ' + $reason),
        ('- Error details: `runner-error.json`')
    ) -join [Environment]::NewLine

    [pscustomobject]@{
        Failure = $failure
        FailureRows = $failureRows
        Summary = $summary
        ErrorDetail = $errorDetail
        SummaryMarkdown = $summaryMarkdown
    }
}

function Write-QualificationRunnerFailureArtifacts {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [object]$Artifacts,
        [Parameter(Mandatory)] [string]$EvidenceRoot,
        [Parameter(Mandatory)] [AllowEmptyCollection()] [System.Collections.Generic.List[string]]$CommandRows
    )

    $Artifacts.Summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'summary.json') -Encoding UTF8
    $Artifacts.FailureRows | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'failures.json') -Encoding UTF8
    $Artifacts.ErrorDetail | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'runner-error.json') -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $EvidenceRoot 'summary.md') -Value $Artifacts.SummaryMarkdown -Encoding UTF8
    $CommandRows | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'commands.txt') -Encoding UTF8
}

Export-ModuleMember -Function ConvertTo-DispatchQualificationCollection, New-DispatchSessionRedesignM3FinalPlan, Invoke-DispatchSessionRedesignM3FinalPlan, Invoke-DispatchSessionRedesignM3FinalControlPath, New-QualificationRunnerFailureArtifacts, Write-QualificationRunnerFailureArtifacts
