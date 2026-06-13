// The wandr:avalonia-guest WIT exports — wit-bindgen requires the impl
// classes in this exact (world-derived) namespace. They forward to the
// runtime in WandrAvalonia.Runtime.
namespace GuestWorld.wit.exports.wasi.inputHandlers.v0_0_2;

using WandrAvalonia;

public class FrameHandlerImpl : IFrameHandler
{
    public static void OnFrame(ulong nanos) => Runtime.Frame(nanos);

    public static void OnResize(uint width, uint height)
    {
        // Surface size is re-read from the frame buffer each on-frame.
    }
}

public class PointerHandlerImpl : IPointerHandler
{
    public static void OnPointer(IPointerHandler.PointerEvent ev) => Runtime.Pointer(ev);
}

public class KeyHandlerImpl : IKeyHandler
{
    public static void OnKey(IKeyHandler.KeyEvent ev) => Runtime.Key(ev);
}
