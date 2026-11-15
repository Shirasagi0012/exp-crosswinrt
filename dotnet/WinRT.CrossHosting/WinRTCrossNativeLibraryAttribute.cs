using System;

namespace WinRT.CrossHosting;

/// <summary>
/// Declares a native shared library implementing the WinRT component (and/or <c>winrt-cross-pal</c>)
/// to be auto-loaded and bound by <see cref="WinRTCrossHost"/> on non-Windows platforms.
/// </summary>
[AttributeUsage(AttributeTargets.Assembly, AllowMultiple = true)]
public sealed class WinRTCrossNativeLibraryAttribute : Attribute
{
    /// <summary>
    /// Initializes a new instance declaring a native library name or path.
    /// </summary>
    /// <param name="libraryName">Logical library stem (e.g. <c>rust_component</c>) or full/relative file path.</param>
    /// <param name="hintPath">Optional fallback path if the library is not adjacent to the application.</param>
    public WinRTCrossNativeLibraryAttribute(string libraryName, string? hintPath = null)
    {
        LibraryName = libraryName ?? throw new ArgumentNullException(nameof(libraryName));
        HintPath = string.IsNullOrWhiteSpace(hintPath) ? null : hintPath;
    }

    /// <summary>
    /// Gets the logical native library stem or file path.
    /// </summary>
    public string LibraryName { get; }

    /// <summary>
    /// Gets the optional fallback path used when probing outside the application base directory.
    /// </summary>
    public string? HintPath { get; }
}
