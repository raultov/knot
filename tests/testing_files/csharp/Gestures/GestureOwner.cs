namespace MyApp.Gestures;

/// <summary>
/// Which control owns a device's single gesture role: explicitly off, or a
/// named button. Mirrors a discriminated union.
/// </summary>
public abstract record GestureOwner
{
    private GestureOwner() { }

    /// <summary>Gestures explicitly turned off for this device.</summary>
    public sealed record Off : GestureOwner;

    /// <summary>The named button owns the gesture role.</summary>
    public sealed record Button(int Id) : GestureOwner;

    public static readonly Off OffValue = new();
}
