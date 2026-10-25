import { Page } from "./Page";

export class CdpPage extends Page {
  async deleteCookie(name: string): Promise<void> {
    const pageURL = this.url();
    const url = new URL(pageURL);
    console.log(name, url);
  }
}
