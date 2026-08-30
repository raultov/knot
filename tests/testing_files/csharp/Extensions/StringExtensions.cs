namespace MyApp.Extensions;

public static class StringExtensions
{
    public static string Slugify(this string value)
    {
        string Normalize(string raw) => raw.Trim().ToLowerInvariant();

        return Normalize(value).Replace(' ', '-');
    }
}