// Soft-keyboard wiring (task 47 IME model) — the same shape slint-wandr
// uses: report editor focus to the host via wandr:ui-shell/ime; the
// arbiter raises the keyboard overlay and typed text comes back through
// the regular key-handler export (Exports.KeyHandlerImpl → KeyTextInput).
//
// HeadlessWindowImpl returns null for ITextInputMethodImpl and is
// `internal` (can't be subclassed to provide one), so instead of plugging
// into Avalonia's text-input-method plumbing we poll the focused element
// each on-frame and reconcile. Attach ONCE on focus, detach on blur —
// re-attaching per keystroke would churn the overlay (slint-wandr ignores
// per-keystroke updates for the same reason; the keyboard tracks content
// from the keys it sends).
namespace WandrAvalonia;

using Avalonia.Controls;
using Avalonia.Input;
using GuestWorld.wit.imports.wandr.uiShell.v0_1_0;

internal static class WandrIme
{
    private static TextBox? _editor;

    /// Reconcile keyboard state with the focused element. Call each frame.
    internal static void Sync(IFocusManager? focusManager)
    {
        var focused = focusManager?.GetFocusedElement() as TextBox;
        if (ReferenceEquals(focused, _editor))
            return;

        if (focused is not null)
        {
            var text = focused.Text ?? "";
            var len = (uint)text.Length;
            // Avalonia selection offsets are char indices — the wandr
            // contract's unit (slint-wandr converts from bytes; we don't).
            var a = (uint)Math.Clamp(focused.SelectionStart, 0, text.Length);
            var b = (uint)Math.Clamp(focused.SelectionEnd, 0, text.Length);
            ImeInterop.NotifyEditorAttached(
                InputType(focused),
                focused.Watermark ?? "",
                text,
                Math.Min(a, b),
                Math.Max(a, b));
        }
        else
        {
            ImeInterop.NotifyEditorDetached();
        }
        _editor = focused;
    }

    private static string InputType(TextBox box)
        => box.PasswordChar != '\0' ? "password" : "text";

    /// ESC is the wandr keyboard's hide button (task-47 convention): blur
    /// the editor so Sync() detaches and the overlay tears down. Returns
    /// true if it consumed the key (an editor was focused).
    internal static bool HandleEscape(IFocusManager? focusManager)
    {
        if (_editor is null)
            return false;
        focusManager?.ClearFocus();
        return true;
    }
}
