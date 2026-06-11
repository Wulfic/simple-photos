<#
.SYNOPSIS
    Bump the project version across every file that hard-codes it.

.DESCRIPTION
    A push to a `vX.Y.Z` tag triggers .github/workflows/release.yml, which
    builds & publishes the release. Tagging requires every version-bearing
    file to agree (Cargo, npm, Android, Inno Setup, docs). This script
    updates them all atomically so a release bump is one command.

    Files updated:
      server/Cargo.toml                       package.version
      server/Cargo.lock                       simple-photos-server entry
      web/package.json                        version
      web/package-lock.json                   top-level + named workspace
      android/app/build.gradle.kts            versionName + versionCode (auto +1)
      packaging/windows/simple-photos.iss     #define SP_VERSION fallback
      packaging/README.md                     example iscc command

    By default the script only edits files. Pass -Commit to commit the
    changes, and -Tag to also create the `vX.Y.Z` tag. Pass -Push to push
    both the branch and the tag to `origin`. Push triggers the release
    workflow, which builds & uploads the .deb / .exe / .apk and creates
    the GitHub Release.

.PARAMETER Version
    The new version string. Must be semver (X.Y.Z, optionally with a
    `-prerelease` or `+build` suffix).

.PARAMETER Commit
    Stage all updated files and create a `chore(release): bump version
    to X.Y.Z` commit on the current branch.

.PARAMETER Tag
    Create an annotated tag `vX.Y.Z` pointing at the new commit.
    Implies -Commit.

.PARAMETER Push
    Push the current branch *and* the new tag to `origin`. Implies -Tag.

.PARAMETER DryRun
    Show what would change without modifying any file.

.EXAMPLE
    .\bump-version.ps1 -Version 1.0.1
    Updates files only.

.EXAMPLE
    .\bump-version.ps1 -Version 1.0.1 -Push
    Full release: bumps, commits, tags, pushes (CI takes over from there).
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Version,
    [switch]$Commit,
    [switch]$Tag,
    [switch]$Push,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

# -- Validation --------------------------------------------------------------
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$') {
    Write-Error "Version '$Version' is not a valid semver (X.Y.Z[-pre][+build])."
    exit 1
}
if ($Push)   { $Tag    = $true }
if ($Tag)    { $Commit = $true }

# Run from the repo root regardless of where the user invokes us.
$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $repoRoot

# Refuse to operate on a dirty tree when we're going to commit/tag/push.
if ($Commit -and -not $DryRun) {
    $dirty = & git status --porcelain
    if ($dirty) {
        Write-Error "Working tree is not clean. Commit/stash before running with -Commit/-Tag/-Push."
        $dirty | ForEach-Object { Write-Host "  $_" }
        exit 1
    }
}

function Edit-File {
    param(
        [Parameter(Mandatory)] [string]$Path,
        [Parameter(Mandatory)] [string]$Pattern,
        [Parameter(Mandatory)] [string]$Replacement,
        [int]$ExpectedMatches = 1,
        [switch]$Regex
    )
    if (-not (Test-Path $Path)) {
        throw "File not found: $Path"
    }
    # Read raw to preserve exact byte content / line endings.
    $original = [System.IO.File]::ReadAllText($Path)
    if ($Regex) {
        $matches = [regex]::Matches($original, $Pattern)
    } else {
        $matches = [regex]::Matches($original, [regex]::Escape($Pattern))
    }
    if ($matches.Count -ne $ExpectedMatches) {
        throw "Expected $ExpectedMatches match(es) of '$Pattern' in $Path but found $($matches.Count). Aborting to avoid corrupting unrelated text."
    }
    if ($Regex) {
        $updated = [regex]::Replace($original, $Pattern, $Replacement)
    } else {
        $updated = $original.Replace($Pattern, $Replacement)
    }
    if ($updated -eq $original) {
        Write-Host "  no-op: $Path (already at target value)"
        return
    }
    Write-Host "  updated $Path"
    if (-not $DryRun) {
        # Preserve original byte encoding (UTF-8, no BOM is the common case
        # for this repo). WriteAllText defaults to UTF-8 without BOM.
        [System.IO.File]::WriteAllText($Path, $updated)
    }
}

Write-Host ""
Write-Host "Bumping project version to $Version" -ForegroundColor Cyan
if ($DryRun) { Write-Host "(DRY RUN -- no files will be written)" -ForegroundColor Yellow }
Write-Host ""

# ---------------------------------------------------------------------------
# 1) server/Cargo.toml -- only the [package] version (not deps).
# ---------------------------------------------------------------------------
Edit-File -Path 'server/Cargo.toml' `
          -Pattern '(?m)^version\s*=\s*"[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?"' `
          -Replacement ('version = "{0}"' -f $Version) `
          -Regex

# ---------------------------------------------------------------------------
# 2) server/Cargo.lock -- the simple-photos-server [[package]] entry.
# ---------------------------------------------------------------------------
Edit-File -Path 'server/Cargo.lock' `
          -Pattern '(?ms)(\[\[package\]\]\s*\nname\s*=\s*"simple-photos-server"\s*\nversion\s*=\s*")[^"]+(")' `
          -Replacement ('${1}' + $Version + '${2}') `
          -Regex

# ---------------------------------------------------------------------------
# 3) web/package.json -- top-level version field.
# ---------------------------------------------------------------------------
Edit-File -Path 'web/package.json' `
          -Pattern '(?m)^(\s*)"version":\s*"[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?"' `
          -Replacement ('${1}"version": "' + $Version + '"') `
          -Regex

# ---------------------------------------------------------------------------
# 4) web/package-lock.json -- top-level + the simple-photos-web workspace
#    entry. Both occur very near the top of the file (lines 3 and 9 typically).
# ---------------------------------------------------------------------------
$lockPath = 'web/package-lock.json'
$lock     = [System.IO.File]::ReadAllText($lockPath)
# Replace ONLY the first two occurrences; later ones belong to nested deps.
$rgx      = [regex]'"version":\s*"[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?"'
$matches  = $rgx.Matches($lock)
if ($matches.Count -lt 2) {
    throw "package-lock.json: expected at least 2 'version' fields, found $($matches.Count)."
}
# Splice in reverse order so earlier offsets remain valid.
$updated = $lock
for ($i = 1; $i -ge 0; $i--) {
    $m = $matches[$i]
    $updated = $updated.Substring(0, $m.Index) + ('"version": "' + $Version + '"') + $updated.Substring($m.Index + $m.Length)
}
Write-Host ("  updated {0} (top-level + workspace)" -f $lockPath)
if (-not $DryRun) { [System.IO.File]::WriteAllText($lockPath, $updated) }

# ---------------------------------------------------------------------------
# 5) android/app/build.gradle.kts -- versionName and versionCode (+1).
#    Google Play / Android requires versionCode to be a strictly increasing
#    integer; we read the current value and increment it.
# ---------------------------------------------------------------------------
$gradlePath = 'android/app/build.gradle.kts'
$gradle     = [System.IO.File]::ReadAllText($gradlePath)
$codeMatch  = [regex]::Match($gradle, '(?m)^\s*versionCode\s*=\s*([0-9]+)')
if (-not $codeMatch.Success) {
    throw "Could not find versionCode in $gradlePath."
}
$currentCode = [int]$codeMatch.Groups[1].Value
$nextCode    = $currentCode + 1
Write-Host ("  android versionCode {0} -> {1}" -f $currentCode, $nextCode)
Edit-File -Path $gradlePath `
          -Pattern '(?m)^(\s*versionCode\s*=\s*)[0-9]+' `
          -Replacement ('${1}' + $nextCode) `
          -Regex
Edit-File -Path $gradlePath `
          -Pattern '(?m)^(\s*versionName\s*=\s*")[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?"' `
          -Replacement ('${1}' + $Version + '"') `
          -Regex

# ---------------------------------------------------------------------------
# 6) packaging/windows/simple-photos.iss -- fallback SP_VERSION (used when
#    iscc isn't given /DSP_VERSION=...).
# ---------------------------------------------------------------------------
Edit-File -Path 'packaging/windows/simple-photos.iss' `
          -Pattern '#define\s+SP_VERSION\s+"[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?"' `
          -Replacement ('#define SP_VERSION "' + $Version + '"') `
          -Regex

# ---------------------------------------------------------------------------
# 7) packaging/README.md -- the example `iscc /DSP_VERSION=...` command and
#    the resulting filename hint.
# ---------------------------------------------------------------------------
Edit-File -Path 'packaging/README.md' `
          -Pattern '/DSP_VERSION=[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?' `
          -Replacement ('/DSP_VERSION=' + $Version) `
          -Regex
Edit-File -Path 'packaging/README.md' `
          -Pattern 'simple-photos-[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?-windows-x64-setup\.exe' `
          -Replacement ('simple-photos-' + $Version + '-windows-x64-setup.exe') `
          -Regex

# ---------------------------------------------------------------------------
# Optional commit / tag / push.
# ---------------------------------------------------------------------------
if ($DryRun) {
    Write-Host ""
    Write-Host "Dry run complete. No files written, no commit created." -ForegroundColor Yellow
    return
}

if ($Commit) {
    Write-Host ""
    Write-Host "Creating commit..." -ForegroundColor Cyan
    & git add `
        server/Cargo.toml `
        server/Cargo.lock `
        web/package.json `
        web/package-lock.json `
        android/app/build.gradle.kts `
        packaging/windows/simple-photos.iss `
        packaging/README.md
    if ($LASTEXITCODE -ne 0) { throw "git add failed." }
    & git commit -m ("chore(release): bump version to {0}" -f $Version)
    if ($LASTEXITCODE -ne 0) { throw "git commit failed." }
}

if ($Tag) {
    $tagName = "v$Version"
    Write-Host ""
    Write-Host ("Creating annotated tag {0}..." -f $tagName) -ForegroundColor Cyan
    & git tag -a $tagName -m ("Release {0}" -f $Version)
    if ($LASTEXITCODE -ne 0) { throw "git tag failed." }
}

if ($Push) {
    Write-Host ""
    Write-Host "Pushing branch and tag to origin..." -ForegroundColor Cyan
    & git push origin HEAD
    if ($LASTEXITCODE -ne 0) { throw "git push branch failed." }
    & git push origin ("v$Version")
    if ($LASTEXITCODE -ne 0) { throw "git push tag failed." }
    Write-Host ""
    Write-Host "Done. Release workflow has been triggered:" -ForegroundColor Green
    Write-Host "  https://github.com/Wulfic/simple-photos/actions"
}

Write-Host ""
Write-Host ("Version bumped to {0}" -f $Version) -ForegroundColor Green
