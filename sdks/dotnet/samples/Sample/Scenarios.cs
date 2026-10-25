using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Threading;
using System.Threading.Tasks;
using Hairspring;

namespace HsSample;

/// <summary>The scenarios of <see cref="Program"/>: each returns the program's exit status (0 returns from Main normally).</summary>
internal static class Scenarios
{
    public static readonly IReadOnlyList<string> Names =
    [
        "basic", "merge", "threads", "exit_status", "plain_exit", "return_status", "raises", "halt", "big", "spawn", "conflict",
        "empty", "limits", "explicit", "background", "close_err", "background_report", "err_after_report", "fork", "pool",
        "bad_input",
    ];

    private const string LongMsg = "a deliberately long message so that each catalog line is large and two processes " +
        "writing at once have a real chance of interleaving if a line were ever split across write calls; padding " +
        "padding padding padding padding padding padding padding padding padding padding padding padding padding " +
        "padding padding padding padding padding padding padding padding padding padding padding";
    private const int BigSites = 64;
    private const int BigRounds = 3;
    private const int IdLimit = 128;
    private const int MsgLimit = 512;
    private const int BackgroundRunMillis = 50;
    private const int HaltStatus = 5;
    private const int PlainExitStatus = 7;
    private const int ReturnedStatus = 9;
    private const int ChildFailed = 3;

    public static int Run(string name, string[] args) => name switch
    {
        "basic" => Basic(),
        "merge" => Merge(),
        "threads" => Threads(args.Length > 0 ? int.Parse(args[0], System.Globalization.CultureInfo.InvariantCulture) : 0),
        "exit_status" => ExitStatus(),
        "plain_exit" => PlainExit(),
        "return_status" => ReturnStatus(),
        "raises" => Raises(),
        "halt" => Halt(),
        "big" => Big(),
        "spawn" => Spawn(),
        "conflict" => Conflict(),
        "empty" => Empty(),
        "limits" => Limits(),
        "explicit" => Explicit(),
        "background" => Background(),
        "close_err" => CloseErr(),
        "background_report" => BackgroundReport(),
        "err_after_report" => ErrAfterReport(),
        "fork" => Fork(),
        "pool" => Pool(),
        "bad_input" => BadInput(),
        _ => throw new ArgumentOutOfRangeException(nameof(name), name, "unknown scenario"),
    };

    private static int Basic()
    {
        Hs.Declare(Kind.Reachable, "basic.reach.never", "never reached");
        Hs.Declare(Kind.Unreachable, "basic.unreach.clean", "never reached either");
        Hs.Declare(Kind.AlwaysOrUnreachable, "basic.aou.never", "never evaluated");
        for (var i = 0; i < 5; i++)
        {
            Hs.Always(true, "basic.always.ok", "always holds", [i, 2]);
        }
        Hs.Always(false, "basic.always.bad", "always violated", [7, 8]);
        for (var i = 0; i < 4; i++)
        {
            Hs.Sometimes(i % 2 == 0, "basic.sometimes", "sometimes holds", [i]);
        }
        Hs.Reachable("basic.reach.hit", "is reached");
        Hs.Work(300);
        return 0;
    }

    private static void MergeA(bool ok) => Hs.Sometimes(ok, "merge.shared", "one id, two sites", [1]);

    private static void MergeB(bool ok) => Hs.Sometimes(ok, "merge.shared", "one id, two sites", [2]);

    private static int Merge()
    {
        MergeA(true);
        MergeA(false);
        MergeB(true);
        MergeB(true);
        Hs.Reachable("merge.other", "a second, distinct id");
        return 0;
    }

    private static int Threads(int count)
    {
        Hs.Work(1);
        for (var i = 0; i < count; i++)
        {
            var index = i;
            var worker = new Thread(() =>
            {
                Hs.NameThread("w", index);
                Hs.Sometimes(true, "threads.each", "every thread evaluates", [index]);
            });
            worker.Start();
            worker.Join();
        }
        return 0;
    }

    private static int ExitStatus()
    {
        Hs.Reachable("exit.reach", "reached");
        Hs.Exit(ChildFailed);
        return 0;
    }

    private static int PlainExit()
    {
        Hs.Reachable("plain.reach", "reached");
        Environment.Exit(PlainExitStatus);
        return 0;
    }

    private static int ReturnStatus()
    {
        Hs.Reachable("return.reach", "reached");
        return ReturnedStatus;
    }

    private static int Raises()
    {
        Hs.Reachable("raises.before", "reached before the exception");
        throw new InvalidOperationException("deliberate");
    }

    private static int Halt()
    {
        Hs.Reachable("halt.before", "reached before the halt");
        Posix.Halt(HaltStatus);
        return 0;
    }

    private static int Big()
    {
        for (var round = 0; round < BigRounds; round++)
        {
            for (var site = 0; site < BigSites; site++)
            {
                Hs.Sometimes(round % 2 == 0, "big.site." + site, LongMsg, [round]);
            }
        }
        return 0;
    }

    private static int Spawn()
    {
        Hs.Reachable("spawn.parent", "parent reaches this");
        var start = new ProcessStartInfo(Environment.ProcessPath ?? "dotnet") { UseShellExecute = false };
        start.ArgumentList.Add(typeof(Program).Assembly.Location);
        start.ArgumentList.Add("merge");
        using var child = Process.Start(Hs.Prepare(start))!;
        child.WaitForExit();
        return child.ExitCode == 0 ? 0 : ChildFailed;
    }

    private static int Conflict()
    {
        Hs.Always(true, "conflict.id", "declared as always");
        Hs.Sometimes(true, "conflict.id", "declared as sometimes");
        return 0;
    }

    private static int Empty()
    {
        Hs.Work(1);
        return 0;
    }

    private static int Limits()
    {
        Hs.Reachable(new string('x', IdLimit), new string('m', MsgLimit));
        Hs.Reachable("bad id$with:odd/chars", "quote \" and backslash \\ in the message");
        return 0;
    }

    private static int Explicit()
    {
        Hs.Reachable("explicit.before", "reached before the report");
        Hs.Report();
        Hs.Reachable("explicit.after", "reached after the report");
        return 0;
    }

    /// <summary>A background thread keeps evaluating while the process ends, so the report is taken against live writers.</summary>
    private static int Background()
    {
        var spinner = new Thread(() =>
        {
            for (long i = 0; ; i++)
            {
                Hs.Sometimes(i % 2 == 0, "background.loop", "evaluated while the process exits", [i]);
            }
        })
        { IsBackground = true };
        spinner.Start();
        Thread.Sleep(BackgroundRunMillis);
        return 0;
    }

    /// <summary>The program closes descriptor 2 itself (native code can); the report must still arrive when standard error is a pipe or a terminal.</summary>
    private static int CloseErr()
    {
        Hs.Reachable("close_err.site", "evaluated before and after the close");
        Posix.Close(2);
        Hs.Reachable("close_err.after", "evaluated after the close");
        return 0;
    }

    /// <summary>A background thread reports explicitly while Main returns and the process ends; the report must still be whole.</summary>
    private static int BackgroundReport()
    {
        Hs.Reachable("background_report.site", "reached before the report");
        var reporter = new Thread(Hs.Report) { IsBackground = true };
        reporter.Start();
        return 0;
    }

    /// <summary>The program writes to standard error after an explicit report: those lines must not overwrite the report.</summary>
    private static int ErrAfterReport()
    {
        Hs.Reachable("err_after_report.site", "reached before the report");
        Hs.Report();
        for (var i = 0; i < 20; i++)
        {
            Console.Error.WriteLine("the program writes to standard error after the report, line " + i);
        }
        return 0;
    }

    /// <summary>A forked copy of the process (native code can fork a managed program) never reports; the parent reports once.</summary>
    private static int Fork()
    {
        Hs.Reachable("fork.parent", "reached before the fork");
        var child = Posix.Fork();
        if (child == 0)
        {
            Hs.Report(); // the copy has the parent's state but not its pid: it must write nothing
            Posix.Halt(0);
        }
        Posix.WaitFor(child);
        return 0;
    }

    /// <summary>Tasks on the thread pool: each pool thread that calls the SDK takes a slot of its own.</summary>
    private static int Pool()
    {
        Hs.Reachable("pool.main", "reached on the main thread");
        Task.Run(() => Hs.Reachable("pool.task", "reached on a pool thread")).Wait();
        return 0;
    }

    /// <summary>Arguments the SDK refuses throw and count nothing, also for a site that is already known.</summary>
    private static int BadInput()
    {
        Hs.Always(true, "input.ok", "fine");
        var refused = 0;
        foreach (var attempt in new Action[]
        {
            () => Hs.Always(true, "input.ok", new string('m', MsgLimit + 1)),
            () => Hs.Always(true, "a\0b", "nul in the id"),
            () => Hs.Always(true, "", "empty id"),
            () => Hs.Always(true, "input.ok", "five values", [1, 2, 3, 4, 5]),
            () => Hs.Declare(Kind.Always, "input.ok", new string('m', MsgLimit + 1)),
        })
        {
            try
            {
                attempt();
            }
            catch (ArgumentException)
            {
                refused++;
            }
        }
        Hs.Always(true, "input.ok", "fine again");
        Console.Out.WriteLine("refused " + refused);
        return 0;
    }
}
