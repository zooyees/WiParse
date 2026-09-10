param(
  [Parameter(Mandatory = $true)]
  [string]$StatusFile
)

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class HudNative {
  [DllImport("user32.dll")] public static extern bool ReleaseCapture();
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hWnd, int msg, int wParam, int lParam);
}
"@

function Format-Elapsed([object]$sec) {
  if ($null -eq $sec -or "$sec" -eq "") { return "" }
  $s = 0
  try { $s = [int]$sec } catch { return "" }
  if ($s -lt 0) { $s = 0 }
  return ("{0}:{1:D2}" -f [int][Math]::Floor($s / 60), ($s % 60))
}

function Get-TriggerText($j) {
  $labels = New-Object System.Collections.Generic.List[string]
  if ($j.PSObject.Properties.Name -contains "triggers" -and $j.triggers) {
    foreach ($t in @($j.triggers)) {
      if ($t -is [string]) { [void]$labels.Add($t) }
      elseif ($t.label) { [void]$labels.Add([string]$t.label) }
      elseif ($t.id) { [void]$labels.Add([string]$t.id) }
    }
  }
  if ($labels.Count -eq 0 -and $j.trigger) { [void]$labels.Add([string]$j.trigger) }
  return (($labels | Select-Object -First 4) -join "  ")
}

$form = New-Object System.Windows.Forms.Form
$form.Text = "ScopeSerial"
$form.TopMost = $true
$form.ShowInTaskbar = $false
$form.FormBorderStyle = "None"
$form.StartPosition = "Manual"
$form.Size = New-Object System.Drawing.Size(348, 46)
$form.BackColor = [System.Drawing.Color]::FromArgb(48, 52, 58)
$form.Padding = New-Object System.Windows.Forms.Padding(1)
$wa = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
$form.Location = New-Object System.Drawing.Point(($wa.Right - $form.Width - 18), ($wa.Top + 14))

$inner = New-Object System.Windows.Forms.Panel
$inner.Dock = "Fill"
$inner.BackColor = [System.Drawing.Color]::FromArgb(250, 250, 250)
$form.Controls.Add($inner)

$strip = New-Object System.Windows.Forms.Panel
$strip.Dock = "Left"
$strip.Width = 5
$strip.BackColor = [System.Drawing.Color]::FromArgb(97, 97, 97)
$inner.Controls.Add($strip)

$btnClose = New-Object System.Windows.Forms.Label
$btnClose.Text = "x"
$btnClose.Dock = "Right"
$btnClose.Width = 22
$btnClose.TextAlign = "MiddleCenter"
$btnClose.Font = New-Object System.Drawing.Font("Segoe UI", 9)
$btnClose.ForeColor = [System.Drawing.Color]::FromArgb(120, 120, 120)
$btnClose.Cursor = [System.Windows.Forms.Cursors]::Hand
$btnClose.Add_Click({ $form.Close() })
$btnClose.Add_MouseEnter({ $btnClose.ForeColor = [System.Drawing.Color]::FromArgb(40, 40, 40) })
$btnClose.Add_MouseLeave({ $btnClose.ForeColor = [System.Drawing.Color]::FromArgb(120, 120, 120) })
$inner.Controls.Add($btnClose)

$body = New-Object System.Windows.Forms.Panel
$body.Dock = "Fill"
$body.Padding = New-Object System.Windows.Forms.Padding(8, 5, 4, 5)
$inner.Controls.Add($body)

$row = New-Object System.Windows.Forms.TableLayoutPanel
$row.Dock = "Fill"
$row.ColumnCount = 3
$row.RowCount = 1
$row.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle([System.Windows.Forms.SizeType]::AutoSize)))
$row.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle([System.Windows.Forms.SizeType]::Percent, 100)))
$row.ColumnStyles.Add((New-Object System.Windows.Forms.ColumnStyle([System.Windows.Forms.SizeType]::AutoSize)))
$body.Controls.Add($row)

$lblState = New-Object System.Windows.Forms.Label
$lblState.AutoSize = $true
$lblState.Font = New-Object System.Drawing.Font("Microsoft YaHei UI", 10, [System.Drawing.FontStyle]::Bold)
$lblState.ForeColor = [System.Drawing.Color]::FromArgb(28, 28, 28)
$lblState.Text = "..."
$lblState.Margin = New-Object System.Windows.Forms.Padding(0, 4, 8, 0)
$row.Controls.Add($lblState, 0, 0)

$lblMeta = New-Object System.Windows.Forms.Label
$lblMeta.Dock = "Fill"
$lblMeta.Font = New-Object System.Drawing.Font("Microsoft YaHei UI", 8.5)
$lblMeta.ForeColor = [System.Drawing.Color]::FromArgb(90, 90, 90)
$lblMeta.TextAlign = "MiddleLeft"
$lblMeta.Text = ""
$lblMeta.Margin = New-Object System.Windows.Forms.Padding(0, 2, 4, 0)
$row.Controls.Add($lblMeta, 1, 0)

$lblTime = New-Object System.Windows.Forms.Label
$lblTime.AutoSize = $true
$lblTime.Font = New-Object System.Drawing.Font("Consolas", 9)
$lblTime.ForeColor = [System.Drawing.Color]::FromArgb(70, 70, 70)
$lblTime.Text = ""
$lblTime.Margin = New-Object System.Windows.Forms.Padding(0, 5, 2, 0)
$row.Controls.Add($lblTime, 2, 0)

$drag = {
  if ($_.Button -eq [System.Windows.Forms.MouseButtons]::Left) {
    [HudNative]::ReleaseCapture() | Out-Null
    [HudNative]::SendMessage($form.Handle, 0xA1, 2, 0) | Out-Null
  }
}
$form.Add_MouseDown($drag)
$inner.Add_MouseDown($drag)
$body.Add_MouseDown($drag)
$lblState.Add_MouseDown($drag)
$lblMeta.Add_MouseDown($drag)
$lblTime.Add_MouseDown($drag)
$strip.Add_MouseDown($drag)

$palette = @{
  wait       = @{ strip = [System.Drawing.Color]::FromArgb(46, 125, 50);  bg = [System.Drawing.Color]::FromArgb(245, 249, 245) }
  processing = @{ strip = [System.Drawing.Color]::FromArgb(230, 81, 0);   bg = [System.Drawing.Color]::FromArgb(255, 248, 240) }
  captured   = @{ strip = [System.Drawing.Color]::FromArgb(21, 101, 192); bg = [System.Drawing.Color]::FromArgb(244, 248, 252) }
  stopped    = @{ strip = [System.Drawing.Color]::FromArgb(198, 40, 40);  bg = [System.Drawing.Color]::FromArgb(252, 244, 244) }
  armed      = @{ strip = [System.Drawing.Color]::FromArgb(84, 110, 122); bg = [System.Drawing.Color]::FromArgb(250, 250, 250) }
}

$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 300
$timer.add_Tick({
  try {
    if (-not (Test-Path -LiteralPath $StatusFile)) { return }
    $j = Get-Content -LiteralPath $StatusFile -Raw -Encoding UTF8 | ConvertFrom-Json
    $step = [string]$j.step
    if (-not $palette.ContainsKey($step)) { $step = "armed" }
    $p = $palette[$step]
    $strip.BackColor = $p.strip
    $inner.BackColor = $p.bg
    $body.BackColor = $p.bg
    $row.BackColor = $p.bg
    $lblState.BackColor = $p.bg
    $lblMeta.BackColor = $p.bg
    $lblTime.BackColor = $p.bg
    $btnClose.BackColor = $p.bg

    $cycle = $j.cycle
    $trig = Get-TriggerText $j
    $time = Format-Elapsed $j.elapsed_s
    switch ([string]$j.step) {
      "wait" {
        $lblState.Text = "监控"
        $lblMeta.Text = ("#{0}  {1}" -f $cycle, $trig)
        $lblTime.Text = $time
      }
      "processing" {
        $lblState.Text = "处理"
        $who = if ($j.trigger) { [string]$j.trigger } else { $trig }
        $lblMeta.Text = ("#{0}  {1}" -f $cycle, $who)
        $lblTime.Text = ""
      }
      "captured" {
        $lblState.Text = "完成"
        $fn = if ($j.filename) { [string]$j.filename } else { "下一轮" }
        $lblMeta.Text = $fn
        $lblTime.Text = ""
      }
      "stopped" {
        $lblState.Text = "停止"
        $lblMeta.Text = ""
        $lblTime.Text = ""
      }
      default {
        $lblState.Text = "准备"
        $lblMeta.Text = if ($j.hint) { [string]$j.hint } else { "" }
        $lblTime.Text = ""
      }
    }
  } catch {}
})
$timer.Start()
[void]$form.ShowDialog()
