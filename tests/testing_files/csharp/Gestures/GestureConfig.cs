namespace MyApp.Gestures;

public class DeviceEntry
{
    public GestureOwner? Owner { get; set; }
}

public class GestureConfig
{
    /// <summary>`is` pattern against a nested record (short qualified path).</summary>
    public bool GesturesEnabled(DeviceEntry d) => !(d.Owner is GestureOwner.Off);

    /// <summary>switch arm with a fully qualified constant pattern.</summary>
    public int? OwnerOf(DeviceEntry d) => d.Owner switch
    {
        MyApp.Gestures.GestureOwner.Off => null,
        GestureOwner.Button b => b.Id,
        _ => null,
    };

    /// <summary>Static field access through a fully qualified path.</summary>
    public void Disable(DeviceEntry d) => d.Owner = MyApp.Gestures.GestureOwner.OffValue;

    /// <summary>Object creation of a nested record.</summary>
    public void Select(DeviceEntry d, int id) => d.Owner = new GestureOwner.Button(id);
}
