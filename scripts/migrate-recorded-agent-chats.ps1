# HalluScribe - one-off migration from Gemma-branded recorded chats to generic agent chats.
# Preserves stable session IDs and raw transcript links; creates a recoverable backup first.
[CmdletBinding(SupportsShouldProcess)]
param(
  [string]$ArchiveRoot = (Join-Path $env:USERPROFILE '.halluscribe')
)

$ErrorActionPreference = 'Stop'
$legacyProvider = 'halluscribe_gemma_chat'
$agentProvider = 'halluscribe_agent_chat'
$legacyRoot = Join-Path $ArchiveRoot 'recorded_sessions\HalluScribe\gemma4'
$agentRoot = Join-Path $ArchiveRoot 'recorded_sessions\HalluScribe\agent-chat'
$indexPath = Join-Path $ArchiveRoot 'index.json'
$settingsPath = Join-Path $ArchiveRoot 'settings.json'

if (!(Test-Path -LiteralPath $legacyRoot)) {
  throw "No legacy recorded chats found under $legacyRoot"
}
if (!(Test-Path -LiteralPath $indexPath)) {
  throw "Archive index is missing: $indexPath"
}

$records = @(Get-ChildItem -LiteralPath $legacyRoot -Recurse -File -Filter '*-gemma4-chat.json' | ForEach-Object {
  $date = $_.Directory.Name
  $newName = $_.Name -replace 'gemma4-chat\.json$', 'agent-chat.json'
  $destination = Join-Path (Join-Path $agentRoot $date) $newName
  if (Test-Path -LiteralPath $destination) {
    throw "Destination already exists: $destination"
  }
  [PSCustomObject]@{ Source = $_.FullName; Destination = $destination }
})
if ($records.Count -eq 0) {
  throw "No legacy recorded chat JSON files found under $legacyRoot"
}

$index = Get-Content -LiteralPath $indexPath -Raw | ConvertFrom-Json
$indexed = @($index.sessions | Where-Object { $_.provider -eq $legacyProvider })
$summaryMoves = @()
foreach ($entry in $indexed) {
  $record = $records | Where-Object { $_.Source -eq $entry.source_jsonl }
  if (!$record) {
    continue
  }
  $oldSummary = Join-Path $ArchiveRoot $entry.archive_path
  $newArchivePath = $entry.archive_path -replace 'gemma4-sweep\.md$', 'agent-chat-sweep.md'
  $newSummary = Join-Path $ArchiveRoot $newArchivePath
  if ($newArchivePath -eq $entry.archive_path -or !(Test-Path -LiteralPath $oldSummary)) {
    throw "Cannot safely rename summary for indexed session $($entry.id)"
  }
  if (Test-Path -LiteralPath $newSummary) {
    throw "Destination summary already exists: $newSummary"
  }
  $summaryMoves += [PSCustomObject]@{
    Entry = $entry; Source = $oldSummary; Destination = $newSummary; ArchivePath = $newArchivePath
  }
}

Write-Output "Recorded agent-chat migration plan: $($records.Count) source files, $($summaryMoves.Count) indexed summaries."
$records | ForEach-Object { Write-Output "  source: $($_.Source) -> $($_.Destination)" }
$summaryMoves | ForEach-Object { Write-Output "  summary: $($_.Source) -> $($_.Destination)" }
if (!$PSCmdlet.ShouldProcess($ArchiveRoot, 'Migrate recorded Gemma chats to generic agent chats')) {
  return
}

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$backupRoot = Join-Path $ArchiveRoot "migration-backups\recorded-agent-chats-$stamp"
New-Item -ItemType Directory -Path $backupRoot -Force | Out-Null
Copy-Item -LiteralPath $indexPath -Destination (Join-Path $backupRoot 'index.before.json')
if (Test-Path -LiteralPath $settingsPath) {
  Copy-Item -LiteralPath $settingsPath -Destination (Join-Path $backupRoot 'settings.before.json')
}
foreach ($record in $records) {
  $relative = $record.Source.Substring($ArchiveRoot.Length).TrimStart('\')
  $backup = Join-Path $backupRoot (Join-Path 'sources' $relative)
  New-Item -ItemType Directory -Path (Split-Path -Parent $backup) -Force | Out-Null
  Copy-Item -LiteralPath $record.Source -Destination $backup
}
foreach ($summary in $summaryMoves) {
  $relative = $summary.Source.Substring($ArchiveRoot.Length).TrimStart('\')
  $backup = Join-Path $backupRoot (Join-Path 'summaries' $relative)
  New-Item -ItemType Directory -Path (Split-Path -Parent $backup) -Force | Out-Null
  Copy-Item -LiteralPath $summary.Source -Destination $backup
}

foreach ($record in $records) {
  $payload = Get-Content -LiteralPath $record.Source -Raw | ConvertFrom-Json
  $payload.tool = 'HalluScribe Agent'
  $payload.provider = $agentProvider
  Set-Content -LiteralPath $record.Source -Value ($payload | ConvertTo-Json -Depth 20) -Encoding utf8
  New-Item -ItemType Directory -Path (Split-Path -Parent $record.Destination) -Force | Out-Null
  Move-Item -LiteralPath $record.Source -Destination $record.Destination
}

foreach ($summary in $summaryMoves) {
  $text = Get-Content -LiteralPath $summary.Source -Raw
  $text = $text.Replace('**Tool:** Gemma 4', '**Tool:** HalluScribe Agent')
  $text = $text.Replace("**Provider:** $legacyProvider", "**Provider:** $agentProvider")
  $text = $text.Replace($summary.Entry.source_jsonl, ($records | Where-Object { $_.Source -eq $summary.Entry.source_jsonl }).Destination)
  Set-Content -LiteralPath $summary.Source -Value $text -Encoding utf8
  New-Item -ItemType Directory -Path (Split-Path -Parent $summary.Destination) -Force | Out-Null
  Move-Item -LiteralPath $summary.Source -Destination $summary.Destination
  $summary.Entry.tool = 'HalluScribe Agent'
  $summary.Entry.provider = $agentProvider
  $summary.Entry.source_jsonl = ($records | Where-Object { $_.Source -eq $summary.Entry.source_jsonl }).Destination
  $summary.Entry.archive_path = $summary.ArchivePath
}

$settings = Get-Content -LiteralPath $settingsPath -Raw | ConvertFrom-Json
$sources = @($settings.profile_sources | ForEach-Object { if ($_ -eq $legacyProvider) { $agentProvider } else { $_ } } | Select-Object -Unique)
if ($sources -notcontains $agentProvider) {
  $sources += $agentProvider
}
$settings.profile_sources = $sources
Set-Content -LiteralPath $settingsPath -Value ($settings | ConvertTo-Json -Depth 20) -Encoding utf8
Set-Content -LiteralPath $indexPath -Value ($index | ConvertTo-Json -Depth 20) -Encoding utf8

Write-Output "Migration complete. Backup: $backupRoot"
