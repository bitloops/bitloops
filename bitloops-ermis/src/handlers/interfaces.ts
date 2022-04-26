import { Services } from "../services/definitions";

export type SubscribeHandlerType = (data, subject: string) => void;

export type ConnectionSubscribeHandlerType = (services: Services, connectionId: string) => SubscribeHandlerType;

export interface ISubscriptionEvents {
    subscribe(topic: string, handler: SubscribeHandlerType);
    connectionTopicSubscribeHandler: ConnectionSubscribeHandlerType;
}