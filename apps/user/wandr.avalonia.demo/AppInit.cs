// Register this app's Application + root Window with avalonia-wandr. A
// [ModuleInitializer] runs at component startup, before the first frame, so
// the library's runtime has the factories ready when on-frame first fires.
namespace AvaloniaDemo;

using System.Runtime.CompilerServices;

internal static class AppInit
{
    [ModuleInitializer]
    internal static void Init()
        => WandrAvalonia.Host.Configure(() => new DemoApp(), DemoApp.BuildMainWindow);
}
