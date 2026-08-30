namespace MyApp.Legacy
{
    namespace Deep
    {
        public delegate void Notifier(string message);

        public class OldStyle
        {
            public event Notifier? OnNotify;

            private readonly string[] _items = new string[10];

            public string this[int index]
            {
                get => _items[index];
                set => _items[index] = value;
            }

            public static OldStyle operator +(OldStyle a, OldStyle b) => a;

            public class Inner
            {
                public int Depth { get; set; }
            }
        }
    }
}