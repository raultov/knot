/** Overload calls bind by arity within the receiver class (issue #38). */
export class Foo {
  bar(a: number): number;
  bar(a: number, b: number): number;
  bar(a: number, b?: number): number {
    return a + (b ?? 0);
  }
}

/** Same name and arity as Foo.bar in another class. */
export class Other {
  bar(a: number, b: number): number {
    return a - b;
  }
}

export function sumPair(foo: Foo): number {
  return foo.bar(1, 2);
}

export function single(foo: Foo): number {
  return foo.bar(1);
}
