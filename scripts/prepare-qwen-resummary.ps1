# HalluScribe - recoverably prepare one archived session for Qwen re-summarisation.
# Run with -WhatIf first. The app's normal "Run Now" sweep re-creates the summary.
[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory)]
    [string]$SessionId,
    [string]$ArchiveRoot = (Join-Path $env:USERPROFILE '.halluscribe')
)

$ErrorActionPreference = 'Stop'
$archiveRoot = (Resolve-Path -LiteralPath $ArchiveRoot).Path
$indexPath = Join-Path $archiveRoot 'index.json'
if (!(Test-Path -LiteralPath $indexPath)) { throw "Archive index not found: $indexPath" }

$index = Get-Content -LiteralPath $indexPath -Raw | ConvertFrom-Json
$matches = @($index.sessions | Where-Object { $_.id -eq $SessionId })
if ($matches.Count -ne 1) { throw "Expected exactly one index entry for '$SessionId'; found $($matches.Count)." }

$entry = $matches[0]
if (!$entry.source_jsonl -or !(Test-Path -LiteralPath $entry.source_jsonl)) {
    throw "Original source is unavailable; refusing to prepare '$SessionId' for re-summarisation."
}
$summaryPath = Join-Path $archiveRoot $entry.archive_path
if (!(Test-Path -LiteralPath $summaryPath)) { throw "Current summary is missing: $summaryPath" }

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$backupDir = Join-Path $archiveRoot "migration-backups\qwen-resummary-smoke-$stamp-$SessionId"
if ($WhatIfPreference) {
    [pscustomobject]@{
        action = 'would_prepare_one_session'
        session_id = $entry.id
        source_jsonl = $entry.source_jsonl
        summary_to_move = $summaryPath
        backup_dir = $backupDir
        raw_transcript = 'untouched'
    } | Format-List
    return
}

New-Item -ItemType Directory -Path $backupDir | Out-Null
$backupIndex = Join-Path $backupDir 'index.before.json'
$backupSummary = Join-Path $backupDir (Split-Path -Leaf $summaryPath)
Copy-Item -LiteralPath $indexPath -Destination $backupIndex

try {
    $index.sessions = @($index.sessions | Where-Object { $_.id -ne $SessionId })
    $tempIndex = Join-Path $archiveRoot "index.qwen-resummary-$stamp.tmp"
    [System.IO.File]::WriteAllText(
        $tempIndex,
        ($index | ConvertTo-Json -Depth 100),
        [System.Text.UTF8Encoding]::new($false)
    )
    Move-Item -LiteralPath $tempIndex -Destination $indexPath -Force
    Move-Item -LiteralPath $summaryPath -Destination $backupSummary -Force
}
catch {
    Copy-Item -LiteralPath $backupIndex -Destination $indexPath -Force
    throw
}

[pscustomobject]@{
    action = 'prepared_one_session'
    session_id = $entry.id
    backup_dir = $backupDir
    next_step = 'Run the normal Qwen sweep, then verify the recreated indexed summary.'
    raw_transcript = 'untouched'
} | Format-List
