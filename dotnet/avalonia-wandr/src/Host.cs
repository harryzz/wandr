// avalonia-wandr public entry point + the runtime that drives Avalonia
// from the wandr host's frame/input callbacks. A consuming app supplies
// only its Application and root Window via Host.Configure (typically from a
// [ModuleInitializer]); everything else — platform bring-up, the
// wasi:canvas render backend, input mapping, soft-keyboard wiring — lives
// here and in the rest of src/.
namespace WandrAvalonia;

using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Input;
using Avalonia.Threading;
using GuestWorld.wit.exports.wasi.inputHandlers.v0_0_2;

/// What an app registers to run on wandr. Call once before the first frame
/// (a [ModuleInitializer] is the natural place).
public static class Host
{
    internal static Func<Application>? AppFactory;
    internal static Func<Window>? WindowFactory;

    public static void Configure(Func<Application> appFactory, Func<Window> windowFactory)
    {
        AppFactory = appFactory;
        WindowFactory = windowFactory;
    }
}

internal static class Runtime
{
    /// W3C wheel deltas arrive in surface units; Avalonia wants notches.
    private const double ScrollUnitsPerLine = 50.0;

    private static Window? _window;
    private static double _width;
    private static double _height;

    private static void EnsureStarted()
    {
        if (_window is not null)
            return;
        if (Host.AppFactory is null || Host.WindowFactory is null)
            throw new InvalidOperationException(
                "avalonia-wandr: call WandrAvalonia.Host.Configure(...) before the first frame " +
                "(e.g. from a [ModuleInitializer]).");

        AppBuilder.Configure(Host.AppFactory)
            .UseHeadless(new AvaloniaHeadlessPlatformOptions { UseHeadlessDrawing = false })
            .UseRenderingSubsystem(WandrRenderInterface.Initialize, "Wandr")
            .SetupWithoutStarting();

        _window = Host.WindowFactory();
        try { Console.WriteLine("avalonia-wandr: platform + app initialized"); } catch { }
    }

    internal static void Frame(ulong nanos)
    {
        EnsureStarted();
        FrameBridge.BeginFrame();
        try
        {
            if (_width != FrameBridge.LogicalWidth || _height != FrameBridge.LogicalHeight)
            {
                _width = FrameBridge.LogicalWidth;
                _height = FrameBridge.LogicalHeight;
                var firstSize = !_window!.IsVisible;
                _window.Width = _width;
                _window.Height = _height;
                if (firstSize)
                    _window.Show();
            }

            // On-demand: pump layout/animation/input and let the compositor
            // render ONLY if something is dirty (it early-outs otherwise).
            // No forced full repaint — incremental redraw into the retained
            // offscreen is correct and idle-cheap; FrameBridge skips the
            // present when nothing drew (the input-render/mini artifact is
            // handled by the InFrame canvas gate, not by full redraw).
            Dispatcher.UIThread.RunJobs();
            AvaloniaHeadlessPlatform.ForceRenderTimerTick();

            // Reconcile the soft keyboard with editor focus (post-input).
            WandrIme.Sync(_window.FocusManager);
        }
        catch (Exception ex)
        {
            FrameBridge.WarnOnce($"frame exception: {ex}");
        }
        finally
        {
            FrameBridge.EndFrame();
        }
    }

    private static RawInputModifiers Modifiers(bool alt, bool ctrl, bool meta, bool shift,
        IPointerHandler.Buttons buttons = default)
    {
        var m = RawInputModifiers.None;
        if (alt) m |= RawInputModifiers.Alt;
        if (ctrl) m |= RawInputModifiers.Control;
        if (meta) m |= RawInputModifiers.Meta;
        if (shift) m |= RawInputModifiers.Shift;
        if ((buttons & IPointerHandler.Buttons.PRIMARY) != 0) m |= RawInputModifiers.LeftMouseButton;
        if ((buttons & IPointerHandler.Buttons.SECONDARY) != 0) m |= RawInputModifiers.RightMouseButton;
        if ((buttons & IPointerHandler.Buttons.MIDDLE) != 0) m |= RawInputModifiers.MiddleMouseButton;
        return m;
    }

    internal static void Pointer(IPointerHandler.PointerEvent ev)
    {
        if (_window is null)
            return;

        // Host pointer coords are physical pixels; Avalonia lives in
        // logical units (density baked into the canvas, RenderScaling 1).
        var density = FrameBridge.Density;
        var point = new Point(ev.x / density, ev.y / density);
        var modifiers = Modifiers(ev.alt, ev.ctrl, ev.meta, ev.shift, ev.buttons);
        var button = ev.button switch
        {
            IPointerHandler.Button.SECONDARY => MouseButton.Right,
            IPointerHandler.Button.MIDDLE => MouseButton.Middle,
            IPointerHandler.Button.BACK => MouseButton.XButton1,
            IPointerHandler.Button.FORWARD => MouseButton.XButton2,
            _ => MouseButton.Left,
        };

        try
        {
            switch (ev.kind)
            {
                case IPointerHandler.Kind.DOWN:
                    _window.MouseDown(point, button, modifiers);
                    break;
                case IPointerHandler.Kind.UP:
                    _window.MouseUp(point, button, modifiers);
                    break;
                case IPointerHandler.Kind.MOVE:
                case IPointerHandler.Kind.ENTER:
                    _window.MouseMove(point, modifiers);
                    break;
                case IPointerHandler.Kind.SCROLL:
                    _window.MouseWheel(point,
                        new Vector(-ev.scrollDx / ScrollUnitsPerLine, -ev.scrollDy / ScrollUnitsPerLine),
                        modifiers);
                    break;
                // leave/cancel: nothing to synthesize
            }
        }
        catch (Exception ex)
        {
            FrameBridge.WarnOnce($"pointer exception: {ex}");
        }
    }

    internal static void Key(IKeyHandler.KeyEvent ev)
    {
        if (_window is null)
            return;

        var modifiers = Modifiers(ev.alt, ev.ctrl, ev.meta, ev.shift);
        try
        {
            // ESC = the wandr keyboard's hide button (task-47 convention):
            // blur the editor instead of forwarding the key.
            if (ev.down && ev.code == "Escape" &&
                WandrIme.HandleEscape(_window.FocusManager))
                return;

            if (TryMapPhysicalKey(ev.code, out var physicalKey))
            {
                if (ev.down)
                    _window.KeyPressQwerty(physicalKey, modifiers);
                else
                    _window.KeyReleaseQwerty(physicalKey, modifiers);
            }

            if (ev.down && ev.text.Length > 0 && !ev.ctrl && !ev.meta &&
                !char.IsControl(ev.text[0]))
            {
                _window.KeyTextInput(ev.text);
            }
        }
        catch (Exception ex)
        {
            FrameBridge.WarnOnce($"key exception: {ex}");
        }
    }

    private static bool TryMapPhysicalKey(string code, out PhysicalKey key)
    {
        // W3C UIEvents code tokens are PhysicalKey names, except letters
        // carry a "Key" prefix ("KeyA" → A).
        var name = code.Length == 4 && code.StartsWith("Key", StringComparison.Ordinal)
            ? code[3..]
            : code;
        return Enum.TryParse(name, ignoreCase: false, out key);
    }
}
