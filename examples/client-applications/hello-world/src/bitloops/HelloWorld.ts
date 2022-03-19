/**
 * @generated automatically by Bitloops
 */
import Bitloops from 'bitloops';

export namespace HelloWorld {

  export type SayHelloRequest = {
    name: string;
  }

  export interface IHelloWorldClient {
    sayHello(input: SayHelloRequest): Promise<[response: any | null, error: any | null]>;
  }

  export class HelloWorldClient implements IHelloWorldClient {
    bitloopsApp: Bitloops;

    constructor(bitloopsApp: Bitloops) {
      this.bitloopsApp = bitloopsApp;
    }

    async sayHello(input: SayHelloRequest): Promise<[response: null | any, error: any | null]> {
      try {
        const response: any = await this.bitloopsApp.request(
          '63ff00ad-131c-4cfe-b795-9a1db2b6a805',
          '1f894fb2-40c9-4e37-8a50-3b5e6624bfb1',
          input,
        );
        return [response, null];
      } catch (error) {
        console.error(error);
        return [null, error];
      }
    }

  }

}
