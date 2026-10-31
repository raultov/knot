export class BasketStore {
  items: number[] = [];
  loading = false;

  get total(): number { return this.items.length; }

  reset(): void {
    this.items = [];
    this.loading = false;
    this.refresh();
  }

  refresh(): void {}

  summary(): number { return this.total + 1; }
}
