namespace MyApp.Overloads.Calls;

/// <summary>Overload calls must bind within the receiver class (issue #35).</summary>
public class Formatter
{
    public string Format(string text, int width) { return text; }

    public string Format(string text) { return text; }
}

public class BaseScaler
{
    public int Scale(int a, int b) { return a * b; }
}

public class ChildScaler : BaseScaler
{
    public int Scale(int a) { return a; }
}

public class Report
{
    public string Wide(Formatter formatter) { return formatter.Format("x", 80); }

    public string Narrow(Formatter formatter) { return formatter.Format("x"); }

    public int ScalePair(ChildScaler childScaler) { return childScaler.Scale(2, 3); }
}
