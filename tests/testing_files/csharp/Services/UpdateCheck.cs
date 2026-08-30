namespace MyApp.Services;

/// <summary>Decides whether an update banner should be shown.</summary>
public class UpdateCheck
{
    /// <summary>Contains "Off" only inside the parameter type name.</summary>
    public bool IsEligible(DateTimeOffset publishedAt, DateTimeOffset now) => publishedAt <= now;

    /// <summary>Name starts with "Off" but is unrelated to GestureOwner.Off.</summary>
    public bool OfflineSlot(int slot) => slot < 0;

    public bool Evaluate(DateTimeOffset now) => IsEligible(now, now) || OfflineSlot(1);
}
