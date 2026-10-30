export class ProductCardComponent {
  snapshot = 'unrelated field';

  render(): string {
    return this.snapshot;
  }
}

export class PromoBanner {
  coupon = 'unrelated field';
}
