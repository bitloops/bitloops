/**
 * @generated automatically by Bitloops
 */
import Bitloops from 'bitloops';

export namespace DemoChat {

  export type Subscription_ChatDemoNewPublicMessage = {
    message: string;
    senderUid: string;
    senderNickname: string;
    sendAt: number;
  }

  export type ChatDemoPublicMessageSentPayload = {
    message: string;
    nickname: string;
    senderUid: string;
  }

  export interface IDemoChatClient {
    chatDemoPublicMessageSent(input: ChatDemoPublicMessageSentPayload): Promise<[response: void | null, error: any | null]>;
  }

  export class DemoChatClient implements IDemoChatClient {
    bitloopsApp: Bitloops;
    Events: {
      ChatDemoNewPublicMessage: () => string,
    };

    constructor(bitloopsApp: Bitloops) {
      this.bitloopsApp = bitloopsApp;
      this.Events = {
        ChatDemoNewPublicMessage: () => 'workflow-events.chat-demo:newPublicMessage',
      }
    }

    async chatDemoPublicMessageSent(input: ChatDemoPublicMessageSentPayload): Promise<[response: void | null, error: any | null]> {
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
