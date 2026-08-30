namespace MyApp.Domain;

public record UserDto(int Id, string Name);

public record struct Coord(double Lat, double Lon);

public struct Point : IEquatable<Point>
{
    public int X { get; init; }
    public int Y { get; init; }

    public bool Equals(Point other) => X == other.X && Y == other.Y;
}

public enum UserStatus
{
    Active,
    Suspended,
    Deleted
}

public class User
{
    public int Id { get; set; }
    public string Name { get; set; } = string.Empty;
}

public class Container
{
    public class Nested
    {
        public int Value { get; set; }
    }
}