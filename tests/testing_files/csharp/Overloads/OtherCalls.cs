namespace MyApp.Overloads.Other;

/// <summary>Same name and arity as Formatter.Format / BaseScaler.Scale in another class.</summary>
public class OtherFormatter
{
    public string Format(string text, int width) { return text; }

    public int Scale(int a, int b) { return a - b; }
}
