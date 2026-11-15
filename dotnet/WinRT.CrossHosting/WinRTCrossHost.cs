using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using WindowsRuntime;

[assembly: InternalsVisibleTo("WinRT.CrossHosting.Tests")]

namespace WinRT.CrossHosting;

/// <summary>
/// Bootstraps CsWinRT 3.0 (<c>WinRT.Runtime</c>) on Linux and macOS by routing Windows API Set
/// P/Invokes (<c>api-ms-win-core-winrt-*</c>, <c>api-ms-win-core-com-*</c>) to registered native
/// libraries exporting <c>winrt-cross-pal</c>.
/// </summary>
public static class WinRTCrossHost
{
    private static readonly object SyncLock = new();
    private static readonly ConcurrentDictionary<string, IntPtr> LibraryHandles = new(StringComparer.OrdinalIgnoreCase);
    private static readonly ConcurrentDictionary<Assembly, byte> HookedAssemblies = new();
    private static readonly ConcurrentDictionary<Assembly, byte> ScannedAssemblies = new();

    private static bool _initialized;
    private static IntPtr _palHandle;

    /// <summary>
    /// Gets whether the cross-platform WinRT host has initialized its resolvers.
    /// </summary>
    public static bool IsInitialized => Volatile.Read(ref _initialized);

    /// <summary>
    /// Gets the primary native handle serving WinRT platform ABI symbols on non-Windows platforms.
    /// </summary>
    public static IntPtr PalHandle => _palHandle;

    /// <summary>
    /// Automatically initializes the host when <c>WinRT.CrossHosting</c> is loaded.
    /// </summary>
#pragma warning disable CA2255 // Library intentionally installs WinRT.Runtime DllImportResolver at assembly load time.
    [ModuleInitializer]
#pragma warning restore CA2255
    internal static void ModuleInit()
    {
        Initialize();
    }

    /// <summary>
    /// Ensures the <c>WinRT.Runtime</c> <see cref="DllImportResolver"/> is registered and scans
    /// currently loaded assemblies for <see cref="WinRTCrossNativeLibraryAttribute"/> declarations.
    /// Safe and idempotent; does nothing on Windows.
    /// </summary>
    public static void Initialize()
    {
        if (OperatingSystem.IsWindows())
        {
            Volatile.Write(ref _initialized, true);
            return;
        }

        lock (SyncLock)
        {
            if (!_initialized)
            {
                try
                {
                    NativeLibrary.SetDllImportResolver(typeof(WindowsRuntimeObject).Assembly, ResolveWinRtPlatformImport);
                }
                catch (InvalidOperationException)
                {
                    // Another resolver was already installed on WinRT.Runtime; keep existing registration.
                }

                AppDomain.CurrentDomain.AssemblyLoad += OnAssemblyLoaded;
                _initialized = true;
            }
        }

        ScanLoadedAssemblies();
    }

    /// <summary>
    /// Registers and loads a native shared library implementing WinRT components / <c>winrt-cross-pal</c>.
    /// </summary>
    /// <param name="libraryNameOrPath">Logical library name (e.g. <c>rust_component</c>) or file path.</param>
    /// <param name="hintPath">Optional fallback path if probing the base directory fails.</param>
    /// <param name="contextAssembly">Optional managed assembly whose directory and P/Invokes should be associated.</param>
    /// <returns>The loaded native library handle.</returns>
    public static IntPtr RegisterNativeLibrary(
        string libraryNameOrPath,
        string? hintPath = null,
        Assembly? contextAssembly = null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(libraryNameOrPath);
        Initialize();

        string normalizedStem = NormalizeStem(libraryNameOrPath);
        if (LibraryHandles.TryGetValue(normalizedStem, out IntPtr existing))
        {
            if (contextAssembly is not null)
            {
                EnsureAssemblyResolver(contextAssembly);
            }
            return existing;
        }

        if (!TryLoadLibrary(libraryNameOrPath, hintPath, contextAssembly, out IntPtr handle))
        {
            throw new DllNotFoundException(
                $"[WinRT.CrossHosting] Unable to load native WinRT library '{libraryNameOrPath}'" +
                (hintPath is null ? "." : $" (hintPath='{hintPath}')."));
        }

        RegisterHandle(handle, normalizedStem);
        RegisterHandle(handle, Path.GetFileName(libraryNameOrPath));
        if (contextAssembly is not null)
        {
            EnsureAssemblyResolver(contextAssembly);
        }

        return handle;
    }

    /// <summary>
    /// Registers an already-loaded native library handle for WinRT PAL dispatch and optional P/Invoke resolution.
    /// </summary>
    public static void RegisterHandle(IntPtr handle, string? libraryAlias = null)
    {
        if (handle == IntPtr.Zero)
        {
            throw new ArgumentException("Native library handle cannot be zero.", nameof(handle));
        }

        Initialize();
        Interlocked.CompareExchange(ref _palHandle, handle, IntPtr.Zero);

        if (!string.IsNullOrWhiteSpace(libraryAlias))
        {
            string stem = NormalizeStem(libraryAlias);
            LibraryHandles[libraryAlias] = handle;
            LibraryHandles[stem] = handle;
        }
    }

    /// <summary>
    /// Overrides the primary native handle used for <c>api-ms-win-core-winrt-*</c> / <c>api-ms-win-core-com-*</c>.
    /// </summary>
    public static void SetPalHandle(IntPtr handle)
    {
        if (handle == IntPtr.Zero)
        {
            throw new ArgumentException("PAL handle cannot be zero.", nameof(handle));
        }

        Initialize();
        Volatile.Write(ref _palHandle, handle);
    }

    /// <summary>
    /// Installs a <see cref="DllImportResolver"/> on <paramref name="assembly"/> so direct P/Invokes
    /// (such as Raw C ABI exports) targeting registered native libraries resolve seamlessly.
    /// </summary>
    public static void EnsureAssemblyResolver(Assembly assembly)
    {
        ArgumentNullException.ThrowIfNull(assembly);
        if (OperatingSystem.IsWindows() || !HookedAssemblies.TryAdd(assembly, 0))
        {
            return;
        }

        try
        {
            NativeLibrary.SetDllImportResolver(assembly, ResolveRegisteredLibraryImport);
        }
        catch (InvalidOperationException)
        {
            // Ignore if the assembly already registered its own resolver.
        }
    }

    /// <summary>
    /// Attempts to look up a previously registered native library handle by name or stem.
    /// </summary>
    public static bool TryGetHandle(string libraryName, out IntPtr handle)
    {
        if (LibraryHandles.TryGetValue(libraryName, out handle))
        {
            return true;
        }

        string stem = NormalizeStem(libraryName);
        return LibraryHandles.TryGetValue(stem, out handle);
    }

    private static void OnAssemblyLoaded(object? sender, AssemblyLoadEventArgs args)
    {
        ScanAssembly(args.LoadedAssembly);
    }

    private static void ScanLoadedAssemblies()
    {
        foreach (Assembly assembly in AppDomain.CurrentDomain.GetAssemblies())
        {
            ScanAssembly(assembly);
        }
    }

    private static void ScanAssembly(Assembly assembly)
    {
        if (assembly.IsDynamic || !ScannedAssemblies.TryAdd(assembly, 0))
        {
            return;
        }

        object[] attrs;
        try
        {
            attrs = assembly.GetCustomAttributes(typeof(WinRTCrossNativeLibraryAttribute), inherit: false);
        }
        catch
        {
            return;
        }

        if (attrs.Length == 0)
        {
            return;
        }

        EnsureAssemblyResolver(assembly);
        foreach (WinRTCrossNativeLibraryAttribute attr in attrs)
        {
            RegisterNativeLibrary(attr.LibraryName, attr.HintPath, assembly);
        }
    }

    private static IntPtr ResolveWinRtPlatformImport(string libraryName, Assembly assembly, DllImportSearchPath? searchPath)
    {
        if (IsWinRtPlatformApiSet(libraryName))
        {
            IntPtr handle = Volatile.Read(ref _palHandle);
            if (handle == IntPtr.Zero)
            {
                ScanLoadedAssemblies();
                handle = Volatile.Read(ref _palHandle);
            }
            return handle;
        }

        return IntPtr.Zero;
    }

    private static IntPtr ResolveRegisteredLibraryImport(string libraryName, Assembly assembly, DllImportSearchPath? searchPath)
    {
        if (TryGetHandle(libraryName, out IntPtr handle))
        {
            return handle;
        }

        return IntPtr.Zero;
    }

    internal static bool IsWinRtPlatformApiSet(string libraryName)
    {
        return libraryName.StartsWith("api-ms-win-core-winrt-", StringComparison.OrdinalIgnoreCase)
            || libraryName.StartsWith("api-ms-win-core-com-", StringComparison.OrdinalIgnoreCase)
            || libraryName.Equals("kernel32", StringComparison.OrdinalIgnoreCase)
            || libraryName.Equals("kernel32.dll", StringComparison.OrdinalIgnoreCase)
            || libraryName.Equals("oleaut32", StringComparison.OrdinalIgnoreCase)
            || libraryName.Equals("oleaut32.dll", StringComparison.OrdinalIgnoreCase);
    }

    private static bool TryLoadLibrary(
        string libraryNameOrPath,
        string? hintPath,
        Assembly? contextAssembly,
        out IntPtr handle)
    {
        if (File.Exists(libraryNameOrPath) && NativeLibrary.TryLoad(libraryNameOrPath, out handle))
        {
            return true;
        }

        foreach (string candidate in EnumerateCandidatePaths(libraryNameOrPath, hintPath, contextAssembly))
        {
            if (File.Exists(candidate) && NativeLibrary.TryLoad(candidate, out handle))
            {
                return true;
            }
        }

        if (!string.IsNullOrWhiteSpace(hintPath) && File.Exists(hintPath) && NativeLibrary.TryLoad(hintPath, out handle))
        {
            return true;
        }

        Assembly anchor = contextAssembly ?? typeof(WinRTCrossHost).Assembly;
        return NativeLibrary.TryLoad(libraryNameOrPath, anchor, DllImportSearchPath.AssemblyDirectory | DllImportSearchPath.ApplicationDirectory, out handle)
            || NativeLibrary.TryLoad(libraryNameOrPath, out handle);
    }

    private static IEnumerable<string> EnumerateCandidatePaths(string libraryName, string? hintPath, Assembly? contextAssembly)
    {
        string fileName = Path.GetFileName(libraryName);
        string stem = NormalizeStem(fileName);
        string platformFile = GetPlatformSharedLibraryName(stem);

        var searchDirs = new List<string>(2);
        if (!string.IsNullOrWhiteSpace(AppContext.BaseDirectory))
        {
            searchDirs.Add(AppContext.BaseDirectory);
            searchDirs.Add(Path.Combine(AppContext.BaseDirectory, "runtimes", RuntimeInformation.RuntimeIdentifier, "native"));
        }

        if (!string.IsNullOrWhiteSpace(hintPath))
        {
            yield return hintPath;
            string? hintDir = Path.GetDirectoryName(hintPath);
            if (!string.IsNullOrWhiteSpace(hintDir))
            {
                searchDirs.Add(hintDir);
            }
        }

        foreach (string dir in searchDirs)
        {
            yield return Path.Combine(dir, fileName);
            yield return Path.Combine(dir, platformFile);
        }
    }

    internal static string NormalizeStem(string libraryNameOrPath)
    {
        string file = Path.GetFileName(libraryNameOrPath);
        if (file.EndsWith(".so", StringComparison.OrdinalIgnoreCase))
        {
            file = file[..^3];
        }
        else if (file.EndsWith(".dylib", StringComparison.OrdinalIgnoreCase))
        {
            file = file[..^6];
        }
        else if (file.EndsWith(".dll", StringComparison.OrdinalIgnoreCase))
        {
            file = file[..^4];
        }

        if (!OperatingSystem.IsWindows() && file.StartsWith("lib", StringComparison.Ordinal) && file.Length > 3)
        {
            file = file[3..];
        }

        return file;
    }

    private static string GetPlatformSharedLibraryName(string stem)
    {
        if (OperatingSystem.IsWindows())
        {
            return stem + ".dll";
        }
        if (OperatingSystem.IsMacOS())
        {
            return "lib" + stem + ".dylib";
        }
        return "lib" + stem + ".so";
    }
}
