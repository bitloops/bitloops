import { Services } from "../services/definitions";

export type SubscribeHandlerType = (data, subject: string) => void;

export type ConnectionSubscribeHandlerType = (services: Services, subscriptionEvents: ISubscriptionEvents, connectionId: string) => SubscribeHandlerType;

export interface ISubscriptionEvents {
    subscribe(topic: string, handler: SubscribeHandlerType);
}