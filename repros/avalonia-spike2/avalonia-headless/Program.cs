// Task 107 / Avalonia spike #2 part B — does Avalonia-proper initialize,
// build a visual tree, lay out and tick a frame inside a wasi-wasm
// NativeAOT component? Headless drawing (the in-tree zero-Skia backend),
// single-threaded render timer, manual dispatcher pumping.
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Themes.Fluent;
using Avalonia.Threading;

Console.WriteLine("avalonia-headless: configuring AppBuilder");
AppBuilder.Configure<App>()
    // 12.0.4's headless RenderTimer is DispatcherTimer-based
    // (RunsInBackground=false) — single-threaded, wasi-safe.
    .UseHeadless(new AvaloniaHeadlessPlatformOptions { UseHeadlessDrawing = true })
    .SetupWithoutStarting();
Console.WriteLine("avalonia-headless: platform initialized");

var button = new Button { Content = "Hello from Avalonia on wasi" };
var panel = new StackPanel { Margin = new Thickness(20), Children = { button } };
var window = new Window { Width = 500, Height = 1000, Content = panel };
window.Show();
Dispatcher.UIThread.RunJobs();
Console.WriteLine($"avalonia-headless: layout done — window {window.Bounds}, button {button.Bounds}");

AvaloniaHeadlessPlatform.ForceRenderTimerTick();
Dispatcher.UIThread.RunJobs();
Console.WriteLine("avalonia-headless: frame ticked OK");

class App : Application
{
    public override void Initialize()
    {
        Styles.Add(new FluentTheme());
    }
}
