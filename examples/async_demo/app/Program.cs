using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using AsyncDemo;
using Windows.Foundation;

Console.WriteLine("============================================================");
Console.WriteLine("  WinRT-on-Linux Async Demo (C# Task <-> Rust Tokio)        ");
Console.WriteLine("============================================================");

var worker = new AsyncWorker();

// 1. Await Rust Tokio sleep (`IAsyncAction`)
var sw = Stopwatch.StartNew();
await worker.DelayAsync(50);
sw.Stop();
Console.WriteLine($"[1] DelayAsync(50ms) awaited in {sw.ElapsedMilliseconds} ms");

// 2. Await Rust CPU/blocking calculation (`IAsyncOperation<ulong>`)
ulong fib50 = await worker.FibonacciAsync(50);
Console.WriteLine($"[2] FibonacciAsync(50) = {fib50}");

// 3. Cancel a long-running Rust Tokio task via CancellationToken -> IAsyncInfo.Cancel()
using (var cts = new CancellationTokenSource(TimeSpan.FromMilliseconds(40)))
{
    var cancelSw = Stopwatch.StartNew();
    try
    {
        await worker.DelayAsync(10_000).AsTask(cts.Token);
        throw new InvalidOperationException("Expected TaskCanceledException");
    }
    catch (OperationCanceledException)
    {
        cancelSw.Stop();
        Console.WriteLine($"[3] DelayAsync(10s) canceled cleanly in {cancelSw.ElapsedMilliseconds} ms");
    }
}

// 4. Incremental progress reporting (`IAsyncOperationWithProgress<string, uint>`)
var progressSteps = new List<uint>();
var progress = new Progress<uint>(step =>
{
    lock (progressSteps)
    {
        progressSteps.Add(step);
    }
});
string summary = await worker.ProcessStepsAsync(4, 15).AsTask(progress);
Console.WriteLine($"[4] ProcessStepsAsync => \"{summary}\" (progress callbacks: [{string.Join(", ", progressSteps)}])");

// 5. Bidirectional async: C# Tasks -> IAsyncOperation<int> -> awaited concurrently via tokio::join! in Rust
var leftOp = Task.Run(async () =>
{
    await Task.Delay(30);
    return 19;
}).AsAsyncOperation();

var rightOp = Task.Run(async () =>
{
    await Task.Delay(30);
    return 23;
}).AsAsyncOperation();

int sum = await worker.SumPairAsync(leftOp, rightOp);
Console.WriteLine($"[5] SumPairAsync(19, 23) via tokio::join! = {sum}");

// 6. Sync vs Async HRESULT exception propagation
try
{
    _ = worker.FailAsync(immediate: true);
    throw new InvalidOperationException("Expected ArgumentException");
}
catch (ArgumentException ex)
{
    Console.WriteLine($"[6a] Sync FailAsync(true) threw ArgumentException (0x{ex.HResult:X8})");
}

try
{
    _ = await worker.FailAsync(immediate: false);
    throw new InvalidOperationException("Expected DivideByZeroException / COMException");
}
catch (COMException ex) when (ex.HResult == unchecked((int)0x80020012))
{
    Console.WriteLine($"[6b] Async FailAsync(false) threw COMException DISP_E_DIVBYZERO (0x{ex.HResult:X8})");
}
catch (DivideByZeroException ex)
{
    Console.WriteLine($"[6b] Async FailAsync(false) threw DivideByZeroException (0x{ex.HResult:X8})");
}

Console.WriteLine("All async interop checks completed successfully!");
