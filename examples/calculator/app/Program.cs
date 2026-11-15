using System;
using System.Runtime.InteropServices;
using CalculatorDemo;

Console.WriteLine("============================================================");
Console.WriteLine("  WinRT-on-Linux Calculator Example (C# <-> Rust)           ");
Console.WriteLine("============================================================");

// 1. Static WinRT method call
double quick = Calculator.QuickEval(CalcOp.Multiply, 6, 7);
Console.WriteLine($"[1] Static QuickEval(6 * 7) = {quick}");

// 2. Parameterized WinRT constructor activation
var calc = new Calculator(initialMemory: 100.0);
Console.WriteLine($"[2] Activated Calculator(100), Memory = {calc.Memory}");

// 3. Instance arithmetic + memory operations
Console.WriteLine($"[3] 12 + 30.5   = {calc.Evaluate(CalcOp.Add, 12, 30.5)}");
Console.WriteLine($"    99 - 42     = {calc.Evaluate(CalcOp.Subtract, 99, 42)}");
Console.WriteLine($"    MemoryAdd(25) => {calc.MemoryAdd(25)}");

// 4. Reverse callback (C# CCW observer invoked from Rust)
var observer = new ConsoleCalcObserver();
calc.EvaluateWithObserver(CalcOp.Divide, 144, 12, observer);

// 5. Zero-copy ReadOnlySpan<double> batch fold in Rust
ReadOnlySpan<double> batch = [2.0, 3.0, 4.0, 5.0];
double product = calc.EvaluateBatch(CalcOp.Multiply, batch);
Console.WriteLine($"[5] EvaluateBatch(Multiply, [2, 3, 4, 5]) = {product}");

// 6. HSTRING return & CoTaskMem-backed CalcRecord[] array return
Console.WriteLine($"[6] LastRecord (HSTRING): {calc.FormatLastRecord()}");
CalcRecord[] history = calc.GetHistory();
Console.WriteLine($"    History count = {history.Length}:");
foreach (var rec in history)
{
    Console.WriteLine($"      - {rec.Op}: ({rec.Left}, {rec.Right}) => {rec.Result}");
}

// 7. WinRT HRESULT -> .NET Exception mapping (DISP_E_DIVBYZERO = 0x80020012)
try
{
    Calculator.QuickEval(CalcOp.Divide, 10, 0);
}
catch (COMException ex) when (ex.HResult == unchecked((int)0x80020012))
{
    Console.WriteLine($"[7] Caught COMException from Rust: 0x{ex.HResult:X8}");
}

calc.MemoryClear();
Console.WriteLine($"[8] Memory after MemoryClear() = {calc.Memory}");

internal sealed class ConsoleCalcObserver : ICalcObserver
{
    public void OnEvaluated(CalcRecord record)
    {
        Console.WriteLine($"[4] [C# CCW Observer] {record.Left} {record.Op} {record.Right} = {record.Result}");
    }
}
