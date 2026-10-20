# Checks the page contract that keeps terminal transfer history out of every page.
# CI-only structural regression test; no desktop runtime or user data is touched.
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
[xml]$page = Get-Content (Join-Path $root 'windows/conduit-ui/MainPage.xaml') -Raw
$code = Get-Content (Join-Path $root 'windows/conduit-ui/MainPage.xaml.cs') -Raw
$viewModel = Get-Content (Join-Path $root 'windows/conduit-ui/ViewModels/MainViewModel.cs') -Raw
$model = Get-Content (Join-Path $root 'windows/conduit-ui/Models/TransferEntry.cs') -Raw

function Assert-Layout([bool]$condition, [string]$message) {
    if (-not $condition) { throw "Desktop transfer UI regression: $message" }
}

$history = $page.SelectSingleNode("//*[local-name()='Grid' and @*[local-name()='Name']='TransfersPage']")
Assert-Layout ($null -ne $history) 'the dedicated history page is missing'
$historyRows = $history.SelectNodes(".//*[local-name()='ItemsRepeater' and @ItemsSource='{Binding Transfers}']")
Assert-Layout ($historyRows.Count -eq 1) 'history must remain accessible in the transfer page'
$allHistoryRows = $page.SelectNodes("//*[local-name()='ItemsRepeater' and @ItemsSource='{Binding Transfers}']")
Assert-Layout ($allHistoryRows.Count -eq 1) 'full persisted history must not render on other pages'

$strip = $page.SelectSingleNode("//*[local-name()='Border' and @Visibility='{Binding ActiveTransfersVisibility}']")
Assert-Layout ($null -ne $strip) 'the live-only transfer strip is missing'
Assert-Layout ($strip.GetAttribute('Grid.Row') -eq '1') 'live strip must be in auto-sized row'
Assert-Layout ($null -ne $strip.SelectSingleNode(".//*[local-name()='ProgressBar' and @Value='{Binding FeaturedTransfer.Percentage}']")) 'live progress must be visible'
Assert-Layout ($null -ne $strip.SelectSingleNode(".//*[local-name()='Button' and @Click='CancelTransfer_Click']")) 'active transfer cancel is missing'
Assert-Layout ($null -ne $strip.SelectSingleNode(".//*[local-name()='Button' and @Click='ShowTransfers_Click']")) 'history entry point is missing'
Assert-Layout (-not ($page.OuterXml.Contains('{Binding TransfersVisibility}'))) 'terminal history still controls a global footer'

$sidebarNodes = $page.SelectNodes("//*[local-name()='Button' and @Click='ShowTransfers_Click']")
Assert-Layout ($sidebarNodes.Count -eq 2) 'sidebar and live strip must both open transfer history'
Assert-Layout ($code.Contains('TransfersPage.Visibility = transfers ? Visibility.Visible : Visibility.Collapsed;')) 'transfer page must be explicitly navigated'
Assert-Layout ($code.Contains('NavigateTo(DesktopSection.Transfers);')) 'navigation handler must open history'

Assert-Layout ($viewModel.Contains('Transfers.FirstOrDefault(item => !item.IsTerminal)')) 'only nonterminal records can occupy the live strip'
Assert-Layout ($viewModel.Contains('OnPropertyChanged(nameof(ActiveTransfersVisibility));')) 'the strip must collapse after the final transfer'
Assert-Layout ($viewModel.Contains('OnPropertyChanged(nameof(FeaturedTransfer));')) 'the highlighted transfer must react to new snapshots'
Assert-Layout ($model.Contains('State is "completed" or "failed" or "cancelled"')) 'finished, failed and cancelled transfers must all be terminal'

Write-Output 'Desktop transfer layout: passed (history page, live-only strip, cancellation, navigation, refresh).'
