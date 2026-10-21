using System.Collections.Generic;

namespace MyApp.Overloads;

/// <summary>Same-named overloads: arity and parameter types pick the override (issue #30).</summary>
public interface INotifier
{
    void Notify(string message);

    void Notify(string message, int priority);

    void Notify(int code);
}

public class EmailNotifier : INotifier
{
    public void Notify(string message) { }

    public void Notify(string message, int priority) { }

    public void Notify(int code) { }
}

/// <summary>Generic supertype: type parameters match concrete arguments.</summary>
public interface IEntryStore<T>
{
    void Put(T entry);

    void Put(List<T> entries);

    void Put(string key, T entry);
}

public class CounterStore : IEntryStore<int>
{
    public void Put(int entry) { }

    public void Put(List<int> entries) { }

    public void Put(string key, int entry) { }
}
