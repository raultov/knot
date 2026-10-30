export class CartStore {
  items: number[] = [];

  update(): void {
    const snapshot = this.items.slice();
    applyDiscount(snapshot, 10);
  }
}

export function logCoupon(coupon: string): void {
  console.log(coupon);
}

export function applyDiscount(values: number[], percent: number): number[] {
  return values.map((value) => value - (value * percent) / 100);
}
