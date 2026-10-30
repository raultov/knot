import { action, computed, makeObservable } from 'mobx';

export class CartStore {
  constructor() { makeObservable(this); }

  helper(): void {}

  @computed
  get total(): number { return 1; }

  get plain(): number { return 2; }

  @action
  reset(): void {}

  update(): void {}
}
