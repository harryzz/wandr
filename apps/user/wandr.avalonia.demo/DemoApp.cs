// The demo UI — code-only (no runtime XAML), Fluent dark, a sampler of
// common controls in a scrollable column, wired with plain event handlers.
namespace AvaloniaDemo;

using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.Media;
using Avalonia.Styling;
using Avalonia.Themes.Fluent;

public class DemoApp : Application
{
    public override void Initialize()
    {
        Styles.Add(new FluentTheme());
        RequestedThemeVariant = ThemeVariant.Dark;
    }

    public static Window BuildMainWindow()
    {
        var clickCount = 0;
        var clickButton = new Button { Content = "Click me: 0", HorizontalAlignment = HorizontalAlignment.Stretch };
        clickButton.Click += (_, _) => clickButton.Content = $"Click me: {++clickCount}";

        var slider = new Slider { Minimum = 0, Maximum = 100, Value = 40 };
        var progress = new ProgressBar { Minimum = 0, Maximum = 100, Value = 40 };
        var sliderLabel = new TextBlock { Text = "Slider: 40", FontSize = 12 };
        slider.PropertyChanged += (_, e) =>
        {
            if (e.Property == Slider.ValueProperty)
            {
                progress.Value = slider.Value;
                sliderLabel.Text = $"Slider: {slider.Value:0}";
            }
        };

        var panel = new StackPanel
        {
            Margin = new Thickness(16),
            Spacing = 10,
            Children =
            {
                new TextBlock
                {
                    Text = "Avalonia on wandr",
                    FontSize = 26,
                    FontWeight = FontWeight.Bold,
                },
                new TextBlock
                {
                    Text = "C# · NativeAOT-LLVM · wasi:canvas — fi ffl ligatures",
                    FontSize = 13,
                    Opacity = 0.7,
                    TextWrapping = TextWrapping.Wrap,
                },
                new Border
                {
                    Height = 6,
                    CornerRadius = new CornerRadius(3),
                    Background = new LinearGradientBrush
                    {
                        StartPoint = new RelativePoint(0, 0, RelativeUnit.Relative),
                        EndPoint = new RelativePoint(1, 0, RelativeUnit.Relative),
                        GradientStops =
                        {
                            new GradientStop(Color.FromRgb(0x4f, 0xc3, 0xf7), 0),
                            new GradientStop(Color.FromRgb(0xab, 0x47, 0xbc), 0.5),
                            new GradientStop(Color.FromRgb(0xff, 0x70, 0x43), 1),
                        },
                    },
                },
                clickButton,
                new CheckBox { Content = "Enable the flux capacitor", IsChecked = true },
                new CheckBox { Content = "Unchecked one" },
                new ToggleSwitch { OnContent = "On", OffContent = "Off", IsChecked = true },
                new RadioButton { Content = "Raster", GroupName = "mode", IsChecked = true },
                new RadioButton { Content = "Vector", GroupName = "mode" },
                sliderLabel,
                slider,
                progress,
                new TextBox { Watermark = "Type here…" },
                new ListBox
                {
                    ItemsSource = new[]
                    {
                        "wasi:canvas 0.0.2", "wasi:input-handlers", "harfbuzz 14.2.1",
                        "componentize-dotnet", "NativeAOT-LLVM",
                    },
                    SelectedIndex = 0,
                    MaxHeight = 190,
                },
            },
        };

        return new Window
        {
            Title = "Avalonia wandr demo",
            Background = new SolidColorBrush(Color.FromRgb(0x1e, 0x25, 0x30)),
            Content = new ScrollViewer { Content = panel },
        };
    }
}
