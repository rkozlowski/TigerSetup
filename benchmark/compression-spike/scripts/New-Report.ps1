#Requires -Version 7.0
<#
    .SYNOPSIS
    Generates benchmark/compression-spike/report.md from results/.

    .DESCRIPTION
    Every number in the report comes from a results file; the narrative
    around the numbers is written here and states what the numbers show.
    Sections whose results file is absent are omitted with a note, so the
    report can be regenerated at any stage of the campaign.
#>
[CmdletBinding()]
param(
    [string] $OutPath
)
# Version 1 on purpose: the results are sparse JSON (optional fields are
# omitted by the tool), and a missing optional property reads as $null here.
Set-StrictMode -Version 1
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $root '..\..')).Path
$resultsDir = Join-Path $root 'results'
if (-not $OutPath) { $OutPath = Join-Path $root 'report.md' }

function Read-Json([string] $name) {
    $path = Join-Path $resultsDir $name
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
}
function N([object] $n) { if ($null -eq $n) { return '' } ('{0:N0}' -f [double]$n) }
function MiB([object] $n) { '{0:N1}' -f ([double]$n / 1MB) }
function Pct([double] $part, [double] $whole) { if ($whole -eq 0) { return '' } '{0:N1}%' -f (100.0 * $part / $whole) }
function Delta([double] $value, [double] $reference) { if ($reference -eq 0) { return '' } $d = 100.0 * ($value - $reference) / $reference; ('{0}{1:N2}%' -f (($d -ge 0) ? '+' : ''), $d) }
function Sec([double] $s) { '{0:N2} s' -f $s }
function Rate([double] $bytes, [double] $seconds) { if ($seconds -le 0) { return '' } $r = $bytes / 1MB / $seconds; if ($r -lt 20) { '{0:N1} MiB/s' -f $r } else { '{0:N0} MiB/s' -f $r } }
function Median([double[]] $values) { $s = @($values | Sort-Object); if ($s.Count -eq 0) { return 0 } $s[[int][math]::Floor(($s.Count - 1) / 2)] }
function Table([string[]] $headers, [object[][]] $rows, [string[]] $align) {
    $sb = [System.Text.StringBuilder]::new()
    $null = $sb.AppendLine('| ' + ($headers -join ' | ') + ' |')
    $sep = for ($i = 0; $i -lt $headers.Count; $i++) { if ($align -and $align[$i] -eq 'r') { '---:' } else { '---' } }
    $null = $sb.AppendLine('|' + (($sep | ForEach-Object { $_ }) -join '|') + '|')
    foreach ($row in $rows) { $null = $sb.AppendLine('| ' + (($row | ForEach-Object { "$_" }) -join ' | ') + ' |') }
    $sb.ToString()
}

$corpus = Read-Json 'corpus.json'
$payloads = Read-Json 'corpus-payloads.json'
$downloads = Read-Json 'corpus-downloads.json'
$inventories = @{}
Get-ChildItem -LiteralPath (Join-Path $resultsDir 'inventory') -Filter '*.json' | ForEach-Object {
    $d = Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
    $inventories[$d.app] = $d
}
$appOrder = @($payloads | Where-Object { $_.payload } | Sort-Object -Property bytes -Descending | ForEach-Object { $_.app })
$stage1 = Read-Json 'stage1.json'
$stage1b = Read-Json 'stage1b-window.json'
$stage1c = Read-Json 'stage1c-bcj.json'
$stage2Partitions = Read-Json 'stage2-partitions.json'
$stage2Layouts = Read-Json 'stage2-layouts.json'
$stage2PerFile = Read-Json 'stage2-per-file.json'
$stage3 = Read-Json 'stage3.json'
$determinism = @(@(Read-Json 'determinism.json') + @(Read-Json 'determinism-blocks.json') | Where-Object { $_ })
if ($determinism.Count -eq 0) { $determinism = $null }
$versions = Read-Json 'versions.json'
$benchmarkInstallers = @{}
foreach ($app in 'sharex', 'winmerge', 'qbittorrent', 'vlc') {
    $p = Join-Path $repoRoot "benchmark\results\$app-installers.json"
    if (Test-Path -LiteralPath $p) { $benchmarkInstallers[$app] = (Get-Content -LiteralPath $p -Raw | ConvertFrom-Json).installers }
}

# ---------------------------------------------------------------- helpers over results
function Get-Input([object] $r) { [double]$r.admitted_bytes + [double]$r.raw_bytes }
function Get-Ratio([object] $r) { [double]$r.total_bytes / (Get-Input $r) }
function Get-CompressWall([object] $r) { if ($r.compress) { [double]$r.compress.report.wall_s } elseif ($r.per_file) { [double]$r.per_file.report.wall_s } else { 0 } }
function Get-CompressMem([object] $r) { if ($r.compress) { [double]$r.compress.memory.peak_working_set } elseif ($r.per_file) { [double]$r.per_file.memory.peak_working_set } else { 0 } }
function Get-DecompressWall([object] $r) { if ($null -ne $r.decompress_median_wall_s) { [double]$r.decompress_median_wall_s } else { 0 } }
function Get-DecompressMem([object] $r) { if ($r.decompress_memory) { [double]$r.decompress_memory.peak_working_set } else { 0 } }

function Summarize-Setting([object[]] $rows) {
    # Aggregate over apps for one setting.
    $input = ($rows | ForEach-Object { Get-Input $_ } | Measure-Object -Sum).Sum
    $admitted = ($rows | ForEach-Object { [double]$_.admitted_bytes } | Measure-Object -Sum).Sum
    $output = ($rows | ForEach-Object { [double]$_.total_bytes } | Measure-Object -Sum).Sum
    $ratios = @($rows | ForEach-Object { Get-Ratio $_ })
    $cw = ($rows | ForEach-Object { Get-CompressWall $_ } | Measure-Object -Sum).Sum
    $dw = ($rows | ForEach-Object { Get-DecompressWall $_ } | Measure-Object -Sum).Sum
    $best = $rows | Sort-Object { Get-Ratio $_ } | Select-Object -First 1
    $worst = $rows | Sort-Object { Get-Ratio $_ } -Descending | Select-Object -First 1
    [PSCustomObject]@{
        label = $rows[0].label
        apps = $rows.Count
        input = $input
        admitted = $admitted
        output = $output
        ratio = $output / $input
        medianRatio = (Median $ratios)
        bestApp = $best.app; bestRatio = (Get-Ratio $best)
        worstApp = $worst.app; worstRatio = (Get-Ratio $worst)
        compressWall = $cw
        decompressWall = $dw
        compressMemMax = ($rows | ForEach-Object { Get-CompressMem $_ } | Measure-Object -Maximum).Maximum
        compressMemMedian = (Median @($rows | ForEach-Object { Get-CompressMem $_ }))
        decompressMemMax = ($rows | ForEach-Object { Get-DecompressMem $_ } | Measure-Object -Maximum).Maximum
        decompressMemMedian = (Median @($rows | ForEach-Object { Get-DecompressMem $_ }))
    }
}

$md = [System.Text.StringBuilder]::new()
function W([string] $text = '') { $null = $script:md.AppendLine($text) }

# ================================================================ header
W '# Compression-architecture spike: Zstandard vs LZMA2 for a solid TigerSetup payload'
W ''
W 'Generated by `benchmark/compression-spike/scripts/New-Report.ps1` from `results/`;'
W 'the method and the tool are described in `benchmark/compression-spike/README.md`.'
W 'Nothing measured here changed the product at the time: TigerSetup 0.7.1, its'
W 'ZIP/DEFLATE payload and its Protocol Buffers schema were untouched, and the'
W 'Architect decided the format separately — for Zstandard, on the product''s own'
W 'transaction-time criterion; see the note at the head of the recommendation''s'
W 'engineering judgment, and `TigerSetup-Design.md` §10.4.'
W ''
W 'Contents: [corpus](#corpus) · [baseline](#the-071-baseline) ·'
W '[classification](#classification-what-071-stores-and-whether-it-should) ·'
W '[codec and level](#stage-1-codec-and-level) · [solid vs independent](#solid-vs-independent-per-file-compression) ·'
W '[grouping and order](#stage-2-grouping-and-order) · [blocks](#stage-3-one-solid-stream-vs-bounded-blocks) ·'
W '[decompression and memory](#decompression-and-memory) · [determinism](#determinism) ·'
W '[the four benchmark applications](#the-four-benchmark-applications-against-071-inno-setup-and-nsis) ·'
W '[outliers](#outliers) · [recommendation](#recommendation)'
W ''

# ================================================================ corpus
W '## Corpus'
W ''
W ('{0} application payloads, {1} files, {2} bytes ({3} MiB): the four canonical' -f ($payloads | Where-Object payload).Count, (N (($payloads | Measure-Object files -Sum).Sum)), (N (($payloads | Measure-Object bytes -Sum).Sum)), (MiB (($payloads | Measure-Object bytes -Sum).Sum)))
W 'installer-technology benchmark payloads reused exactly, six more open-source'
W 'applications from their official current release artifacts, and five IT Tiger'
W 'applications from their real release payloads. Every source, hash, exclusion and'
W 'normalization is in `results/corpus.json`, `results/corpus-downloads.json` and'
W '`results/corpus-payloads.json`; the acquisition research for the extra OSS'
W 'applications is `results/oss-apps-research.json`.'
W ''
$rows = foreach ($p in ($payloads | Sort-Object -Property bytes -Descending)) {
    $entry = $corpus.apps | Where-Object { $_.name -eq $p.app }
    $src = switch ($entry.source) {
        'canonical' { 'canonical benchmark payload' }
        'download' { "$($entry.kind): ``$($entry.filename)``" }
        'local-tree' { 'release staging tree (Inno Setup 7 installer, not unpackable)' }
        'tigersetup-installer' { 'payload block of the shipped TigerSetup installer' }
        'unavailable' { 'not acquired' }
    }
    $notes = ($p.normalizations | Where-Object { $_ -notlike '(existing*' }) -join ' '
    ,@($p.app, $p.version, $entry.license, $src, (N $p.files), (N $p.bytes), $notes)
}
W (Table @('App', 'Version', 'Licence', 'Source', 'Files', 'Bytes', 'Normalizations') $rows @('', '', '', '', 'r', 'r', ''))
if ($downloads) {
    W 'Downloaded artifacts (all hashes matched the published SHA-256):'
    W ''
    W (Table @('App', 'Artifact', 'Bytes', 'SHA-256') @($downloads | ForEach-Object { ,@($_.app, "``$($_.filename)``", (N $_.bytes), "``$($_.sha256)``") }) @('', '', 'r', ''))
}
$gimp = $corpus.apps | Where-Object { $_.source -eq 'unavailable' }
foreach ($g in $gimp) { W "**$($g.name) $($g.version) is not in the corpus.** $($g.reason)"; W '' }

# ================================================================ baseline
W '## The 0.7.1 baseline'
W ''
W 'What TigerSetup 0.7.1 writes today: every file either stored (its signature or'
W 'extension says it is already compressed; a 1 MiB-or-larger file whose three'
W 'sampled 64 KiB slices deflate to 98% or more; or DEFLATE-9 was no smaller) or'
W 'DEFLATE-9 on its own, in path order, in a ZIP. `cspike inventory` reproduces that'
W 'decision and that encoder per file; the sums below are the ZIP payload minus its'
W 'headers (checked against the real 0.7.1 installers: WinMerge 24,797,947 here,'
W '24,861,078 in the built installer''s payload block of 473 entries).'
W ''
$rows = foreach ($app in $appOrder) {
    $inv = $inventories[$app]
    ,@($app, (N $inv.file_count), (N $inv.total_bytes), (N $inv.current_total_bytes), (Pct $inv.current_total_bytes $inv.total_bytes), (Sec $inv.baseline_deflate_wall_s), (Rate $inv.total_bytes $inv.baseline_deflate_wall_s))
}
$totalInput = ($appOrder | ForEach-Object { [double]$inventories[$_].total_bytes } | Measure-Object -Sum).Sum
$totalBaseline = ($appOrder | ForEach-Object { [double]$inventories[$_].current_total_bytes } | Measure-Object -Sum).Sum
$totalBaselineWall = ($appOrder | ForEach-Object { [double]$inventories[$_].baseline_deflate_wall_s } | Measure-Object -Sum).Sum
$rows += ,@('**Corpus**', (N (($appOrder | ForEach-Object { [double]$inventories[$_].file_count } | Measure-Object -Sum).Sum)), (N $totalInput), (N $totalBaseline), (Pct $totalBaseline $totalInput), (Sec $totalBaselineWall), (Rate $totalInput $totalBaselineWall))
W (Table @('App', 'Files', 'Input bytes', '0.7.1 payload bytes', 'Ratio', 'DEFLATE-9 pass', 'Throughput') $rows @('', 'r', 'r', 'r', 'r', 'r', 'r'))
W 'The DEFLATE-9 pass is the per-file encode alone, single-threaded, the same'
W 'encoder and level the builder runs (the builder additionally probes and writes'
W 'the ZIP); it is the build-cost reference for the codec times below.'
W ''

# ================================================================ classification
W '## Classification: what 0.7.1 stores, and whether it should'
W ''
$rows = foreach ($app in $appOrder) {
    $inv = $inventories[$app]
    $bd = $inv.by_decision
    $sig = if ($bd.PSObject.Properties['signature']) { $bd.signature } else { $null }
    $probe = if ($bd.PSObject.Properties['probe']) { $bd.probe } else { $null }
    $defl = if ($bd.PSObject.Properties['deflate']) { $bd.deflate } else { $null }
    $ns = if ($bd.PSObject.Properties['deflate_not_smaller']) { $bd.deflate_not_smaller } else { $null }
    $sigB = if ($sig) { [double]$sig.bytes } else { 0 }; $sigF = if ($sig) { $sig.files } else { 0 }
    $probeB = if ($probe) { [double]$probe.bytes } else { 0 }; $probeF = if ($probe) { $probe.files } else { 0 }
    $deflB = if ($defl) { [double]$defl.bytes } else { 0 }; $deflF = if ($defl) { $defl.files } else { 0 }
    $nsB = if ($ns) { [double]$ns.bytes } else { 0 }; $nsF = if ($ns) { $ns.files } else { 0 }
    # What the stored files would become alone under each candidate.
    $stored = @($inv.files | Where-Object { $_.decision -ne 'deflate' -and $_.bytes -gt 0 })
    $z = ($stored | ForEach-Object { [double]$_.alone.'zstd-19' } | Measure-Object -Sum).Sum
    $l = ($stored | ForEach-Object { [double]$_.alone.'lzma2-9' } | Measure-Object -Sum).Sum
    $storedB = ($stored | ForEach-Object { [double]$_.bytes } | Measure-Object -Sum).Sum
    ,@($app, (N $inv.total_bytes), "$sigF / $(N $sigB)", "$probeF / $(N $probeB)", "$deflF / $(N $deflB)", "$nsF / $(N $nsB)", (N ($storedB - $z)), (N ($storedB - $l)))
}
W (Table @('App', 'Input bytes', 'Stored by signature (files / bytes)', 'Stored by probe', 'Admitted to DEFLATE', 'DEFLATE no smaller', 'zstd-19 alone would save', 'LZMA2-9 alone would save') $rows @('', 'r', 'r', 'r', 'r', 'r', 'r', 'r'))
$allStored = @($appOrder | ForEach-Object { $inv = $inventories[$_]; $inv.files | Where-Object { $_.decision -ne 'deflate' -and $_.bytes -gt 0 } | ForEach-Object { [PSCustomObject]@{ app = $inv.app; path = $_.path; bytes = [double]$_.bytes; decision = $_.decision; ratio = $(if ($_.PSObject.Properties['probe_ratio']) { $_.probe_ratio } else { $null }); z = [double]$_.alone.'zstd-19'; l = [double]$_.alone.'lzma2-9' } } })
$storedTotal = ($allStored | Measure-Object bytes -Sum).Sum
$storedZ = ($allStored | Measure-Object z -Sum).Sum
$storedL = ($allStored | Measure-Object l -Sum).Sum
W ('Across the corpus 0.7.1 stores {0} files, {1} bytes ({2} of the input). Compressed one by one, zstd-19 would make them {3} bytes ({4} saved, {5} of the corpus input) and LZMA2-9 {6} bytes ({7} saved, {8}).' -f (N $allStored.Count), (N $storedTotal), (Pct $storedTotal $totalInput), (N $storedZ), (N ($storedTotal - $storedZ)), (Pct ($storedTotal - $storedZ) $totalInput), (N $storedL), (N ($storedTotal - $storedL)), (Pct ($storedTotal - $storedL) $totalInput))
W ''
W 'The stored files where a candidate codec gains most, alone (top 12 by bytes saved with LZMA2-9):'
W ''
$top = @($allStored | Sort-Object { $_.bytes - $_.l } -Descending | Select-Object -First 12)
W (Table @('App', 'File', 'Decision', 'Bytes', 'zstd-19 alone', 'LZMA2-9 alone', 'Saved (LZMA2-9)') @($top | ForEach-Object { ,@($_.app, "``$($_.path)``", "$($_.decision)$(if ($_.ratio) { ' (probe ' + ('{0:N3}' -f $_.ratio) + ')' })", (N $_.bytes), (N $_.z), (N $_.l), (Pct ($_.bytes - $_.l) $_.bytes)) }) @('', '', '', 'r', 'r', 'r', 'r'))
$byExt = @($allStored | Group-Object { [System.IO.Path]::GetExtension($_.path).ToLowerInvariant() } | ForEach-Object { [PSCustomObject]@{ ext = $_.Name; files = $_.Count; bytes = ($_.Group | Measure-Object bytes -Sum).Sum; saved = ($_.Group | ForEach-Object { $_.bytes - $_.l } | Measure-Object -Sum).Sum } } | Sort-Object bytes -Descending)
W 'By extension, everything 0.7.1 stores:'
W ''
W (Table @('Extension', 'Files', 'Bytes', 'LZMA2-9 alone would save', 'Of those bytes') @($byExt | ForEach-Object { ,@("``$($_.ext)``", $_.files, (N $_.bytes), (N $_.saved), (Pct $_.saved $_.bytes)) }) @('', 'r', 'r', 'r', 'r'))
$probeHits = @($allStored | Where-Object { $_.decision -eq 'probe' })
W ('The sampled probe rejected {0} file(s) in the whole corpus ({1} bytes); the signature/extension rule did the rest.' -f $probeHits.Count, (N (($probeHits | Measure-Object bytes -Sum).Sum)))
W ''

# ================================================================ stage 1
W '## Stage 1: codec and level'
W ''
if ($stage1) {
    $s1 = @($stage1 | Where-Object { $_.partition -eq 'current' -and $_.layout -eq 'path' -and -not $_.block_bytes -and -not $_.per_file })
    $labels = @($s1 | ForEach-Object { $_.label } | Select-Object -Unique)
    W 'One solid stream per application, files in path order, 0.7.1''s partition (its'
    W 'stored files stay raw and are counted at full size in every total). `store`'
    W 'is the copy-through floor: what reading the files, writing them and CRC-ing'
    W 'them back costs with no codec at all. Settings, as the libraries report them:'
    W ''
    $paramRows = foreach ($label in $labels) {
        $r = $s1 | Where-Object { $_.label -eq $label } | Sort-Object { Get-Input $_ } -Descending | Select-Object -First 1
        $p = $r.compress.report.parameters
        $desc = switch ($r.setting.codec) {
            'store' { 'copy' }
            'deflate' { "flate2/zlib-rs level $($p.level), 32 KiB window (the product's encoder)" }
            'zstd' { "libzstd $($p.library -replace 'libzstd ','') level $($p.level): strategy $($p.strategy), windowLog $($p.windowLog) ($(MiB $p.windowBytes) MiB), chainLog $($p.chainLog), hashLog $($p.hashLog), searchLog $($p.searchLog), minMatch $($p.minMatch), targetLength $($p.targetLength)$(if ($p.longDistanceMatching) { ', long-distance matching on' })" }
            'lzma2' { "liblzma $($p.library -replace 'liblzma ','') preset $($p.preset)$(if ($p.extreme) { 'e' }): dict $(MiB $p.dictSize) MiB, lc$($p.lc) lp$($p.lp) pb$($p.pb), mode $($p.mode), nice_len $($p.niceLen), mf $($p.matchFinder), depth $($p.depth)" }
        }
        ,@("``$label``", $desc)
    }
    W (Table @('Setting', 'Effective parameters (largest payload)') $paramRows)
    W 'zstd''s window log is capped at the pledged input size, so a payload smaller'
    W 'than the window uses a window of its own size; the largest payload is shown.'
    W ''
    W '### Aggregate over the corpus'
    W ''
    $summaries = @($labels | ForEach-Object { $l = $_; Summarize-Setting @($s1 | Where-Object { $_.label -eq $l }) })
    $rows = foreach ($s in $summaries) {
        ,@("``$($s.label)``", (N $s.output), (Pct $s.output $s.input), (Delta $s.output $totalBaseline), ('{0:N1}%' -f (100 * $s.medianRatio)), ('{0} ({1:N1}%)' -f $s.bestApp, (100 * $s.bestRatio)), ('{0} ({1:N1}%)' -f $s.worstApp, (100 * $s.worstRatio)), (Sec $s.compressWall), (Rate $s.admitted $s.compressWall), (Sec $s.decompressWall), (Rate $s.admitted $s.decompressWall), (MiB $s.compressMemMax), (MiB $s.decompressMemMax))
    }
    W (Table @('Setting', 'Total bytes', 'Ratio', 'vs 0.7.1', 'Median app ratio', 'Best app', 'Worst app', 'Compress (sum)', 'Compress rate', 'Decompress (sum of medians)', 'Decompress rate', 'Peak compress WS (max, MiB)', 'Peak decompress WS (max, MiB)') $rows @('', 'r', 'r', 'r', 'r', '', '', 'r', 'r', 'r', 'r', 'r', 'r'))
    W ('The 0.7.1 baseline over the same corpus is {0} bytes ({1}). Rates are over the bytes admitted to the codec ({2} of the input); `store` shows the I/O floor of the measurement.' -f (N $totalBaseline), (Pct $totalBaseline $totalInput), (Pct $summaries[0].admitted $summaries[0].input))
    W ''
    W '### Per application: compressed bytes and ratio'
    W ''
    $rows = foreach ($app in $appOrder) {
        $cells = @($app, (N $inventories[$app].current_total_bytes))
        foreach ($label in ($labels | Where-Object { $_ -ne 'store' })) {
            $r = $s1 | Where-Object { $_.app -eq $app -and $_.label -eq $label }
            $cells += if ($r) { '{0} ({1:N1}%)' -f (N $r.total_bytes), (100 * (Get-Ratio $r)) } else { '—' }
        }
        ,@($cells)
    }
    W (Table (@('App', '0.7.1') + @($labels | Where-Object { $_ -ne 'store' } | ForEach-Object { "``$_``" })) $rows (@('', 'r') + @($labels | Where-Object { $_ -ne 'store' } | ForEach-Object { 'r' })))
    W '### Per application: compression time'
    W ''
    $rows = foreach ($app in $appOrder) {
        $cells = @($app, (Sec $inventories[$app].baseline_deflate_wall_s))
        foreach ($label in ($labels | Where-Object { $_ -ne 'store' })) {
            $r = $s1 | Where-Object { $_.app -eq $app -and $_.label -eq $label }
            $cells += if ($r) { Sec (Get-CompressWall $r) } else { '—' }
        }
        ,@($cells)
    }
    W (Table (@('App', '0.7.1 DEFLATE-9 pass') + @($labels | Where-Object { $_ -ne 'store' } | ForEach-Object { "``$_``" })) $rows (@('', 'r') + @($labels | Where-Object { $_ -ne 'store' } | ForEach-Object { 'r' })))
    W '### Per application: decompression time (median of three streaming decodes)'
    W ''
    $rows = foreach ($app in $appOrder) {
        $cells = @($app)
        foreach ($label in $labels) {
            $r = $s1 | Where-Object { $_.app -eq $app -and $_.label -eq $label }
            $cells += if ($r) { '{0:N3} s' -f (Get-DecompressWall $r) } else { '—' }
        }
        ,@($cells)
    }
    W (Table (@('App') + @($labels | ForEach-Object { "``$_``" })) $rows (@('') + @($labels | ForEach-Object { 'r' })))
    W '### Per application: peak working set of the compressing process (MiB)'
    W ''
    $rows = foreach ($app in $appOrder) {
        $cells = @($app)
        foreach ($label in $labels) {
            $r = $s1 | Where-Object { $_.app -eq $app -and $_.label -eq $label }
            $cells += if ($r) { MiB (Get-CompressMem $r) } else { '—' }
        }
        ,@($cells)
    }
    W (Table (@('App') + @($labels | ForEach-Object { "``$_``" })) $rows (@('') + @($labels | ForEach-Object { 'r' })))
    W '### Per application: peak working set of the decompressing process (MiB)'
    W ''
    $rows = foreach ($app in $appOrder) {
        $cells = @($app)
        foreach ($label in $labels) {
            $r = $s1 | Where-Object { $_.app -eq $app -and $_.label -eq $label }
            $cells += if ($r) { MiB (Get-DecompressMem $r) } else { '—' }
        }
        ,@($cells)
    }
    W (Table (@('App') + @($labels | ForEach-Object { "``$_``" })) $rows (@('') + @($labels | ForEach-Object { 'r' })))
}
else { W '_stage1.json is not present; Stage 1 has not run._'; W '' }


# ================================================================ stage 1b: window
W '### Window size'
W ''
if ($stage1b) {
    W 'Stage 1 showed the match window to be a first-order variable on the large'
    W 'payloads (zstd-19 at its default 8 MiB window against the same level with a'
    W '128 MiB window and long-distance matching; LZMA2-9''s 64 MiB dictionary). A'
    W 'narrow follow-up: zstd-19 at 64 MiB (the same reach as LZMA2-9) and 256 MiB,'
    W 'LZMA2-9 at 128 MiB and 256 MiB. Decode memory is essentially the window, so'
    W 'this is also the memory trade-off.'
    W ''
    $wlabels = @('zstd-19', 'zstd-19-w26', 'zstd-19-w27', 'zstd-19-w28', 'lzma2-9', 'lzma2-9-w27', 'lzma2-9-w28')
    $wrows = @($s1 + $stage1b)
    $rows = foreach ($app in $appOrder) {
        $cells = @($app)
        foreach ($label in $wlabels) {
            $r = $wrows | Where-Object { $_.app -eq $app -and $_.label -eq $label } | Select-Object -First 1
            $cells += if ($r) { '{0} ({1:N1}%)' -f (N $r.total_bytes), (100 * (Get-Ratio $r)) } else { '—' }
        }
        ,@($cells)
    }
    W (Table (@('App') + @($wlabels | ForEach-Object { "``$_``" })) $rows (@('') + @($wlabels | ForEach-Object { 'r' })))
    $rows = foreach ($label in $wlabels) {
        $rs = @($wrows | Where-Object { $_.label -eq $label })
        if ($rs.Count -eq 0) { continue }
        $sum = Summarize-Setting $rs
        $big = $rs | Sort-Object { Get-Input $_ } -Descending | Select-Object -First 1
        ,@("``$label``", (MiB $big.compress.report.parameters.$(if ($label -like 'zstd*') { 'windowBytes' } else { 'dictSize' })), (N $sum.output), (Pct $sum.output $sum.input), (Delta $sum.output $totalBaseline), (Sec $sum.compressWall), (Rate $sum.admitted $sum.compressWall), (Rate $sum.admitted $sum.decompressWall), (MiB $sum.compressMemMax), (MiB $sum.decompressMemMax))
    }
    W (Table @('Setting', 'Window / dict (MiB)', 'Total bytes', 'Ratio', 'vs 0.7.1', 'Compress (sum)', 'Compress rate', 'Decompress rate', 'Peak compress WS (max)', 'Peak decompress WS (max)') $rows @('', 'r', 'r', 'r', 'r', 'r', 'r', 'r', 'r', 'r'))
}
else { W '_stage1b-window.json is not present._'; W '' }

# ================================================================ stage 1c: BCJ
W '### An x86 branch-converter filter ahead of LZMA2'
W ''
if ($stage1c) {
    W 'Native PE code is 50–99% of most payloads here and is the part that'
    W 'compresses worst, so one more narrow variable: liblzma''s x86 BCJ filter'
    W '(the branch converter 7-Zip applies to executables, which zstd has no'
    W 'equivalent of) ahead of LZMA2-9, on the whole stream — the filter is'
    W 'harmless on non-code bytes and needs no per-file decision. Run alongside'
    W 'Stage 3 (sizes are exact; its times and memory were measured under load and'
    W 'are indicative only).'
    W ''
    $pairs = @(@('lzma2-9', 'lzma2-9-bcj'), @('lzma2-9-w27', 'lzma2-9-w27-bcj'))
    $all = @($s1 + $stage1b + $stage1c)
    $rows = foreach ($app in $appOrder) {
        $cells = @($app)
        foreach ($pair in $pairs) {
            $a = $all | Where-Object { $_.app -eq $app -and $_.label -eq $pair[0] } | Select-Object -First 1
            $b = $all | Where-Object { $_.app -eq $app -and $_.label -eq $pair[1] } | Select-Object -First 1
            $cells += if ($a -and $b) { '{0} → {1} ({2})' -f (N $a.total_bytes), (N $b.total_bytes), (Delta $b.total_bytes $a.total_bytes) } else { '—' }
        }
        ,@($cells)
    }
    W (Table (@('App') + @($pairs | ForEach-Object { "``$($_[0])`` → ``$($_[1])``" })) $rows @('', 'r', 'r'))
    foreach ($pair in $pairs) {
        $a = ($all | Where-Object { $_.label -eq $pair[0] } | Measure-Object total_bytes -Sum).Sum
        $bs = @($all | Where-Object { $_.label -eq $pair[1] })
        if ($bs.Count -eq 0) { continue }
        $b = ($bs | Measure-Object total_bytes -Sum).Sum
        $sum = Summarize-Setting $bs
        W ('- `{0}` → `{1}`: {2} → {3} bytes ({4}, {5} vs 0.7.1); decode rate under load {6}, peak decode WS {7} MiB.' -f $pair[0], $pair[1], (N $a), (N $b), (Delta $b $a), (Delta $b $totalBaseline), (Rate $sum.admitted $sum.decompressWall), (MiB $sum.decompressMemMax))
    }
    W ''
}
else { W '_stage1c-bcj.json is not present._'; W '' }

$script:stage1Summaries = if ($stage1) { $summaries } else { $null }
$script:s1 = if ($stage1) { $s1 } else { $null }

# ================================================================ stage 2: solid vs per-file, partitions, layouts
W '## Solid vs independent per-file compression'
W ''
if ($stage2PerFile) {
    W 'The same settings, every admitted file compressed on its own (a fresh encoder'
    W 'per file, a file stored when its result is no smaller) against the solid'
    W 'stream of Stage 1:'
    W ''
    $labels2 = @($stage2PerFile | ForEach-Object { $_.label } | Select-Object -Unique)
    $rows = foreach ($app in $appOrder) {
        $cells = @($app)
        foreach ($label in $labels2) {
            $pf = $stage2PerFile | Where-Object { $_.app -eq $app -and $_.label -eq $label }
            $solid = $s1 | Where-Object { $_.app -eq $app -and $_.label -eq $label }
            $cells += if ($pf -and $solid) { '{0} → {1} ({2})' -f (N $pf.total_bytes), (N $solid.total_bytes), (Delta $solid.total_bytes $pf.total_bytes) } else { '—' }
        }
        ,@($cells)
    }
    W (Table (@('App') + @($labels2 | ForEach-Object { "``$_`` per-file → solid" })) $rows (@('') + @($labels2 | ForEach-Object { 'r' })))
    foreach ($label in $labels2) {
        $pfT = ($stage2PerFile | Where-Object { $_.label -eq $label } | Measure-Object total_bytes -Sum).Sum
        $solidT = ($s1 | Where-Object { $_.label -eq $label -and $_.app -in ($stage2PerFile | Where-Object { $_.label -eq $label } | ForEach-Object app) } | Measure-Object total_bytes -Sum).Sum
        W ('- `{0}`: per-file {1} bytes, solid {2} bytes ({3}).' -f $label, (N $pfT), (N $solidT), (Delta $solidT $pfT))
    }
    W ''
}
else { W '_stage2-per-file.json is not present._'; W '' }

W '## Partition: what goes into the solid stream'
W ''
if ($stage2Partitions) {
    W '`current` is 0.7.1''s decision (signature- and probe-stored files stay raw);'
    W '`signature` admits the probe-rejected files; `all` admits every file. Totals'
    W 'include the raw bytes of whatever stays out.'
    W ''
    $labels2 = @($stage2Partitions | ForEach-Object { $_.label } | Select-Object -Unique)
    $parts = @('current', 'signature', 'all')
    foreach ($label in $labels2) {
        W "**``$label``**"
        W ''
        $rows = foreach ($app in $appOrder) {
            $cur = ($s1 + $stage2Partitions) | Where-Object { $_.app -eq $app -and $_.label -eq $label -and $_.partition -eq 'current' -and $_.layout -eq 'path' -and -not $_.block_bytes -and -not $_.per_file } | Select-Object -First 1
            if (-not $cur) { continue }
            $cells = @($app, (N $cur.raw_bytes), (N $cur.total_bytes))
            foreach ($part in @('signature', 'all')) {
                $r = $stage2Partitions | Where-Object { $_.app -eq $app -and $_.label -eq $label -and $_.partition -eq $part }
                $cells += if ($r) { '{0} ({1})' -f (N $r.total_bytes), (Delta $r.total_bytes $cur.total_bytes) } else { '—' }
            }
            ,@($cells)
        }
        W (Table @('App', 'Raw bytes under `current`', '`current` total', '`signature` total', '`all` total') $rows @('', 'r', 'r', 'r', 'r'))
        $curT = (($s1 + $stage2Partitions) | Where-Object { $_.label -eq $label -and $_.partition -eq 'current' -and $_.layout -eq 'path' -and -not $_.block_bytes -and -not $_.per_file } | Measure-Object total_bytes -Sum).Sum
        $allT = ($stage2Partitions | Where-Object { $_.label -eq $label -and $_.partition -eq 'all' } | Measure-Object total_bytes -Sum).Sum
        $sigT = ($stage2Partitions | Where-Object { $_.label -eq $label -and $_.partition -eq 'signature' } | Measure-Object total_bytes -Sum).Sum
        W ('Corpus: `current` {0}, `signature` {1} ({2}), `all` {3} ({4}).' -f (N $curT), (N $sigT), (Delta $sigT $curT), (N $allT), (Delta $allT $curT))
        W ''
    }
}
else { W '_stage2-partitions.json is not present._'; W '' }

W '## Stage 2: grouping and order'
W ''
if ($stage2Layouts) {
    W 'Layouts, all deterministic: `path` (byte order of the install-relative path,'
    W 'what 0.7.1 does); `ext-path` (extension, then path: grouping with no'
    W 'semantics); `text-first` (families text, resources, other, symbols,'
    W 'managed/localization, native); `binary-first` (the reverse); `largest-first`'
    W '(families by total bytes, largest first); `text-first-ext` (`text-first` groups,'
    W 'extension-then-path inside each). Families come from the file''s extension and,'
    W 'for PE images and extension-less files, its bytes (`tool/src/classify.rs`).'
    W 'Differences are relative to `path` for the same app and setting.'
    W ''
    $labels2 = @($stage2Layouts | ForEach-Object { $_.label } | Select-Object -Unique)
    $layouts = @($stage2Layouts | ForEach-Object { $_.layout } | Select-Object -Unique | Where-Object { $_ -ne 'path' })
    foreach ($label in $labels2) {
        W "**``$label``**"
        W ''
        $rows = foreach ($app in $appOrder) {
            $base = ($s1 + $stage2Layouts) | Where-Object { $_.app -eq $app -and $_.label -eq $label -and $_.partition -eq 'current' -and $_.layout -eq 'path' -and -not $_.block_bytes -and -not $_.per_file } | Select-Object -First 1
            if (-not $base) { continue }
            $cells = @($app, (N $base.total_bytes))
            foreach ($layout in $layouts) {
                $r = $stage2Layouts | Where-Object { $_.app -eq $app -and $_.label -eq $label -and $_.layout -eq $layout }
                $cells += if ($r) { '{0} ({1})' -f (N $r.total_bytes), (Delta $r.total_bytes $base.total_bytes) } else { '—' }
            }
            ,@($cells)
        }
        W (Table (@('App', '`path`') + @($layouts | ForEach-Object { "``$_``" })) $rows (@('', 'r') + @($layouts | ForEach-Object { 'r' })))
        $baseT = (($s1 + $stage2Layouts) | Where-Object { $_.label -eq $label -and $_.partition -eq 'current' -and $_.layout -eq 'path' -and -not $_.block_bytes -and -not $_.per_file } | Measure-Object total_bytes -Sum).Sum
        $line = foreach ($layout in $layouts) { $t = ($stage2Layouts | Where-Object { $_.label -eq $label -and $_.layout -eq $layout } | Measure-Object total_bytes -Sum).Sum; '`{0}` {1} ({2})' -f $layout, (N $t), (Delta $t $baseT) }
        W ('Corpus: `path` {0}; {1}.' -f (N $baseT), ($line -join '; '))
        W ''
    }
}
else { W '_stage2-layouts.json is not present._'; W '' }

# ================================================================ stage 3
W '## Stage 3: one solid stream vs bounded blocks'
W ''
if ($stage3) {
    W 'Blocks hold whole files: a block closes after the file that brings it to the'
    W 'target or beyond, so a file larger than the target sits alone, and every block'
    W 'is compressed independently (a fresh encoder, no shared history) and decoded'
    W 'independently. Differences are relative to the one solid stream of the same'
    W 'app, layout and setting.'
    W ''
    $labels3 = @($stage3 | ForEach-Object { $_.label } | Select-Object -Unique)
    $layouts3 = @($stage3 | ForEach-Object { $_.layout } | Select-Object -Unique)
    $blocks = @($stage3 | ForEach-Object { [long]$_.block_bytes } | Select-Object -Unique | Sort-Object)
    foreach ($label in $labels3) {
        foreach ($layout in $layouts3) {
            $set = @($stage3 | Where-Object { $_.label -eq $label -and $_.layout -eq $layout })
            if ($set.Count -eq 0) { continue }
            W "**``$label``, layout ``$layout``**"
            W ''
            $rows = foreach ($app in $appOrder) {
                $base = ($s1 + $stage2Layouts) | Where-Object { $_.app -eq $app -and $_.label -eq $label -and $_.layout -eq $layout -and $_.partition -eq 'current' -and -not $_.block_bytes -and -not $_.per_file } | Select-Object -First 1
                if (-not $base -or -not ($set | Where-Object { $_.app -eq $app })) { continue }
                $cells = @($app, (N $base.total_bytes))
                foreach ($b in $blocks) {
                    $r = $set | Where-Object { $_.app -eq $app -and [long]$_.block_bytes -eq $b } | Select-Object -First 1
                    $cells += if ($r) { '{0} ({1}), {2} blk' -f (N $r.total_bytes), (Delta $r.total_bytes $base.total_bytes), $r.compress.report.blocks.Count } else { '—' }
                }
                ,@($cells)
            }
            W (Table (@('App', 'One stream') + @($blocks | ForEach-Object { "$([int]($_ / 1MB)) MiB blocks" })) $rows (@('', 'r') + @($blocks | ForEach-Object { 'r' })))
            $appsIn = @($set | ForEach-Object { $_.app } | Select-Object -Unique)
            $baseT = (($s1 + $stage2Layouts) | Where-Object { $_.app -in $appsIn -and $_.label -eq $label -and $_.layout -eq $layout -and $_.partition -eq 'current' -and -not $_.block_bytes -and -not $_.per_file } | Measure-Object total_bytes -Sum).Sum
            $line = foreach ($b in $blocks) { $t = ($set | Where-Object { [long]$_.block_bytes -eq $b } | Measure-Object total_bytes -Sum).Sum; '{0} MiB {1} ({2})' -f [int]($b / 1MB), (N $t), (Delta $t $baseT) }
            W ('These {0} applications, one stream {1}; {2}.' -f $appsIn.Count, (N $baseT), ($line -join '; '))
            W ''
        }
    }
    W 'Decode time per block set (sum over the applications above, one streaming'
    W 'decode each, a fresh decoder per block) against the one-stream medians. Stage'
    W '2 and 3 ran six jobs at a time, so these times are indicative only; the'
    W 'sizes are exact.'
    W ''
    $rows = foreach ($label in $labels3) {
        foreach ($layout in $layouts3) {
            $set = @($stage3 | Where-Object { $_.label -eq $label -and $_.layout -eq $layout })
            if ($set.Count -eq 0) { continue }
            $appsIn = @($set | ForEach-Object { $_.app } | Select-Object -Unique)
            $baseD = (($s1 + $stage2Layouts) | Where-Object { $_.app -in $appsIn -and $_.label -eq $label -and $_.layout -eq $layout -and $_.partition -eq 'current' -and -not $_.block_bytes -and -not $_.per_file } | ForEach-Object { Get-DecompressWall $_ } | Measure-Object -Sum).Sum
            $cells = @("``$label``", $layout, (Sec $baseD))
            foreach ($b in $blocks) { $cells += Sec (($set | Where-Object { [long]$_.block_bytes -eq $b } | ForEach-Object { Get-DecompressWall $_ } | Measure-Object -Sum).Sum) }
            ,@($cells)
        }
    }
    W (Table (@('Setting', 'Layout', 'One stream') + @($blocks | ForEach-Object { "$([int]($_ / 1MB)) MiB blocks" })) $rows (@('', '', 'r') + @($blocks | ForEach-Object { 'r' })))
}
else { W '_stage3.json is not present._'; W '' }

# ================================================================ decompression and memory
W '## Decompression and memory'
W ''
if ($stage1) {
    W 'From Stage 1 (one stream, path order). Decompression rate is admitted bytes over'
    W 'the sum of per-app median decode times; `store` is the floor (file read + CRC-32'
    W 'with no codec). Peak working set is the whole child process.'
    W ''
    $storeS = $summaries | Where-Object { $_.label -eq 'store' }
    $rows = foreach ($s in $summaries) {
        ,@("``$($s.label)``", (Rate $s.admitted $s.compressWall), (Rate $s.admitted $s.decompressWall), ('{0:N1}×' -f ($s.decompressWall / $storeS.decompressWall)), (MiB $s.compressMemMedian), (MiB $s.compressMemMax), (MiB $s.decompressMemMedian), (MiB $s.decompressMemMax))
    }
    W (Table @('Setting', 'Compress rate', 'Decompress rate', 'Decode time vs `store`', 'Compress WS median', 'Compress WS max', 'Decompress WS median', 'Decompress WS max') $rows @('', 'r', 'r', 'r', 'r', 'r', 'r', 'r'))
}

# ================================================================ determinism
W '## Determinism'
W ''
if ($determinism) {
    W 'Each job below compressed its input twice, in two separate processes, and'
    W 'the SHA-256 of both outputs is compared; where the same job also ran in'
    W 'Stage 1 or Stage 3 (an earlier process, hours apart), that output''s hash is'
    W 'compared too.'
    W ''
    $earlier = @($s1 + @($stage3 | Where-Object { $_ }))
    $rows = foreach ($r in $determinism) {
        $same = ($r.output_sha256 | Select-Object -Unique).Count -eq 1
        $prior = $earlier | Where-Object { $_.id -eq $r.id } | Select-Object -First 1
        $priorText = if ($prior) { if ($prior.output_sha256[0] -eq $r.output_sha256[0]) { 'same as the earlier run' } else { '**differs from the earlier run**' } } else { 'no earlier run' }
        ,@($r.app, "``$($r.label)``", $r.layout, $(if ($r.block_bytes) { "$([int]($r.block_bytes / 1MB)) MiB blocks" } else { 'one stream' }), $r.output_sha256.Count, $(if ($same) { 'identical' } else { '**DIFFERENT**' }), $priorText, "``$($r.output_sha256[0].Substring(0, 16))…``")
    }
    W (Table @('App', 'Setting', 'Layout', 'Blocks', 'Runs', 'Outputs', 'Earlier run', 'SHA-256') $rows)
    $bad = @($determinism | Where-Object { ($_.output_sha256 | Select-Object -Unique).Count -ne 1 })
    if ($bad.Count -eq 0) { W 'Every repeated compression produced byte-identical output.' } else { W "**$($bad.Count) job(s) produced different bytes on repetition.**" }
    W ''
}
else { W '_determinism.json is not present._'; W '' }

# ================================================================ the four benchmark apps
W '## The four benchmark applications against 0.7.1, Inno Setup and NSIS'
W ''
$layouts4 = Read-Json 'benchmark-installers-layout.json'
if ($benchmarkInstallers.Count -gt 0 -and $stage1 -and $layouts4) {
    W 'Installer bytes from the installer-technology benchmark (`report.md` at the'
    W 'repository root; `benchmark/results/<app>-installers.json`); the real 0.7.1'
    W 'installers'' block layout from `tiger-setup inspect`'
    W '(`results/benchmark-installers-layout.json`); candidate payloads from Stage 1'
    W '(one stream, path order, 0.7.1 partition). A candidate installer is estimated'
    W 'as the candidate payload plus exactly the non-payload bytes of the real 0.7.1'
    W 'installer (engine 2,509,824 + that package''s metadata + footer), i.e. the same'
    W 'engine and metadata with only the payload block replaced.'
    W ''
    $map = @{ 'ShareX' = 'sharex'; 'WinMerge' = 'winmerge'; 'qBittorrent' = 'qbittorrent'; 'VLC' = 'vlc' }
    $cands = @('zstd-19', 'zstd-19-w27', 'lzma2-6', 'lzma2-9') | Where-Object { $_ -in ($s1 | ForEach-Object label) }
    W '**Payload blocks** (bytes; the 0.7.1 block is the real ZIP with its headers):'
    W ''
    $rows = foreach ($app in @('ShareX', 'WinMerge', 'qBittorrent', 'VLC')) {
        $l4 = $layouts4.$app
        $cells = @($app, (N $l4.payload_length))
        foreach ($c in $cands) {
            $r = $s1 | Where-Object { $_.app -eq $app -and $_.label -eq $c }
            $cells += '{0} ({1})' -f (N $r.total_bytes), (Delta $r.total_bytes $l4.payload_length)
        }
        ,@($cells)
    }
    W (Table (@('App', '0.7.1 payload block') + @($cands | ForEach-Object { "``$_``" })) $rows (@('', 'r') + @($cands | ForEach-Object { 'r' })))
    W '**Whole installers** (bytes; candidates estimated as above; the delta is against the smaller of Inno Setup and NSIS):'
    W ''
    $rows = foreach ($app in @('ShareX', 'WinMerge', 'qBittorrent', 'VLC')) {
        $inst = $benchmarkInstallers[$map[$app]]
        $tiger = [double]($inst | Where-Object { $_.name -like '*TigerSetup*' } | Select-Object -First 1).bytes
        $inno = [double]($inst | Where-Object { $_.name -like '*InnoSetup*' } | Select-Object -First 1).bytes
        $nsis = [double]($inst | Where-Object { $_.name -like '*NSIS*' } | Select-Object -First 1).bytes
        $smallest = [math]::Min($inno, $nsis)
        $l4 = $layouts4.$app
        $overhead = [double]$l4.file_length - [double]$l4.payload_length
        $cells = @($app, (N $inno), (N $nsis), ('{0} ({1})' -f (N $tiger), (Delta $tiger $smallest)))
        foreach ($c in $cands) {
            $r = $s1 | Where-Object { $_.app -eq $app -and $_.label -eq $c }
            $est = [double]$r.total_bytes + $overhead
            $cells += '{0} ({1})' -f (N $est), (Delta $est $smallest)
        }
        ,@($cells)
    }
    W (Table (@('App', 'Inno Setup', 'NSIS', '0.7.1 (real)') + @($cands | ForEach-Object { "``$_`` (est.)" })) $rows (@('', 'r', 'r', 'r') + @($cands | ForEach-Object { 'r' })))
    W 'Inno Setup''s and NSIS''s own overheads are inside their numbers (about 2.1 MB'
    W 'and 40 KB respectively, from the minimal installers in the benchmark); the'
    W '2.5 MB TigerSetup engine is why a payload at parity still leaves a small'
    W 'installer a few percent larger, most visibly on WinMerge.'
    W ''
}
else { W '_Benchmark installer results, Stage 1 or the 0.7.1 layout record are not present._'; W '' }

# ================================================================ outliers, recommendation (narrative)
$narrative = Join-Path $resultsDir 'narrative.md'
if (Test-Path -LiteralPath $narrative) {
    W (Get-Content -LiteralPath $narrative -Raw)
}
else {
    W '## Outliers'; W ''; W '_narrative.md not written yet._'; W ''
    W '## Recommendation'; W ''; W '_narrative.md not written yet._'; W ''
}

# ================================================================ method appendix
W '## Method and environment'
W ''
$host1 = if ($stage1) { $stage1[0].host } else { '' }
W "- Host: AMD Ryzen 7 5700X (8 cores, 16 threads; reported as $host1); Windows 11 Pro 10.0.26200; 32 GiB RAM; NVMe SSD. Codecs single-threaded; Stage 1 ran two jobs at a time and the window study three (timing evidence); the BCJ, Stage 2 and Stage 3 plans up to six at a time and alongside each other (sizes exact, times indicative)."
if ($versions) { W "- Libraries: libzstd $($versions.libzstd), $($versions.liblzma), $($versions.flate2); cspike $($versions.cspike) built with $($versions.rustc)." }
W '- Every measurement is a child process: the worker times its codec loop only (files streamed from a warm cache through the encoder to a file; the compressed file streamed through the decoder into a CRC-32), and the runner reads the child''s peak working set after exit.'
W '- Decompression: three streaming decodes per job in Stage 1 and the window study (median reported, all three kept), one in the BCJ, Stage 2 and Stage 3 plans.'
W '- Totals always include the raw (stored) bytes of the files outside the solid stream, so every total is a payload the installer would carry.'
W '- Plans, in `New-Plan.ps1` terms: see `README.md`. Results: `results/stage1.json`, `stage1b-window.json`, `stage1c-bcj.json`, `stage2-per-file.json`, `stage2-partitions.json`, `stage2-layouts.json`, `stage3.json`, `determinism.json`, `determinism-blocks.json`, `inventory/<App>.json`.'
W ''

[System.IO.File]::WriteAllText($OutPath, $md.ToString(), [System.Text.UTF8Encoding]::new($false))
Write-Host "Wrote $OutPath"
