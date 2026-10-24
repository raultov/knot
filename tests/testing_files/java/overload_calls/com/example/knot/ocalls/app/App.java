package com.example.knot.ocalls.app;

import com.example.knot.ocalls.x.Child;
import com.example.knot.ocalls.x.Foo;

public class App {
    public int sumPair(Foo foo) {
        return foo.bar(1, 2);
    }

    public int single(Foo foo) {
        return foo.bar(1);
    }

    public int scalePair(Child child) {
        return child.scale(2, 3);
    }
}
