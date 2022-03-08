import Bitloops from 'bitloops';

export namespace DemoChat {
 
  export type Publish_PublicMessage = {
    message: string;
    nickname: string;
    senderUid: string;
  }

  export type Subscription_NewPublicMessage = { 
    message: string;
    sendAt: number;
    senderNickname: string;
    senderUid: string;
  }
  
  export interface IDemoChatClient {
    publicMessageSent(input: Publish_PublicMessage): Promise<[response: void | null, error: any | null]>;
  }

  export class DemoChatClient implements IDemoChatClient {
    bitloopsApp: Bitloops;
    Events: { NewPublicMessage: () => string };
    constructor(bitloopsApp: Bitloops) {
      this.bitloopsApp = bitloopsApp;
      this.Events = {
        NewPublicMessage: () => 'workflow-events.chat-demo:newPublicMessage',
      }
    }

    /**
     * @generated from Bitloops Protobuf: PublicMessageSent(PublishPublicMessage) returns (google.protobuf.Empty);
     */
    async publicMessageSent(input: Publish_PublicMessage): Promise<[response: void | null, error: any | null]> {
      try {
        const response: void = await this.bitloopsApp.publish(
          'chat-demo.publicMessageSent',
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