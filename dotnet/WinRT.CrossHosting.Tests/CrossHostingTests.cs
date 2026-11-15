using System;
using System.IO;
using System.Runtime.InteropServices;
using WinRT.CrossHosting;
using Xunit;

namespace WinRT.CrossHosting.Tests;

public class CrossHostingTests
{
    [Fact]
    public void ModuleInitializer_AutoInitializesHost()
    {
        Assert.True(WinRTCrossHost.IsInitialized);
    }

    [Theory]
    [InlineData("api-ms-win-core-winrt-l1-1-0.dll", true)]
    [InlineData("api-ms-win-core-winrt-string-l1-1-0.dll", true)]
    [InlineData("api-ms-win-core-com-l1-1-0.dll", true)]
    [InlineData("api-ms-win-core-winrt-error-l1-1-1.dll", true)]
    [InlineData("kernel32.dll", true)]
    [InlineData("kernel32", true)]
    [InlineData("oleaut32.dll", true)]
    [InlineData("libc.so.6", false)]
    public void IsWinRtPlatformApiSet_MatchesExpectedLibraries(string libraryName, bool expected)
    {
        Assert.Equal(expected, WinRTCrossHost.IsWinRtPlatformApiSet(libraryName));
    }

    [Theory]
    [InlineData("rust_component", "rust_component")]
    [InlineData("librust_component.so", "rust_component")]
    [InlineData("/tmp/foo/libbench_rust.so", "bench_rust")]
    [InlineData("libdemo.dylib", "demo")]
    [InlineData("my_lib.dll", "my_lib")]
    public void NormalizeStem_ExtractsLogicalLibraryStem(string input, string expectedStem)
    {
        Assert.Equal(expectedStem, WinRTCrossHost.NormalizeStem(input));
    }

    [Fact]
    public void RegisterNativeLibrary_LoadsExistingRustComponentAndExportsPalSymbols()
    {
        string rustLibPath = Path.GetFullPath(Path.Combine(
            AppContext.BaseDirectory,
            "..", "..", "..", "..", "..", "..", "rust_component", "target", "release", "librust_component.so"));

        Assert.True(File.Exists(rustLibPath), $"Expected built librust_component.so at {rustLibPath}");

        IntPtr handle = WinRTCrossHost.RegisterNativeLibrary("rust_component", rustLibPath, typeof(CrossHostingTests).Assembly);
        Assert.NotEqual(IntPtr.Zero, handle);
        Assert.NotEqual(IntPtr.Zero, WinRTCrossHost.PalHandle);
        Assert.True(WinRTCrossHost.TryGetHandle("rust_component", out IntPtr resolved));
        Assert.Equal(handle, resolved);
        Assert.True(NativeLibrary.TryGetExport(handle, "WindowsCreateString", out _));
        Assert.True(NativeLibrary.TryGetExport(handle, "RoGetActivationFactory", out _));
        Assert.True(NativeLibrary.TryGetExport(handle, "CoTaskMemAlloc", out _));
    }
}
