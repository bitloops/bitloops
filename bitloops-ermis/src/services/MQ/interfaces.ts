import { MQConnection, MQSubscription } from "./definitions";

export interface IMQ {
    initializeConnection(): Promise<MQConnection>;
    getConnection(): Promise<MQConnection>;
    closeConnection(): Promise<void>;
    gracefullyCloseConnection(): Promise<void>;
    publish(topic: string, message: Record<string, unknown> | string): Promise<void>;
    request<T>(topic: string, body: any): Promise<T>;
    subscribe(topic: string, callbackFunction?: (data: any, subject: string) => void, subscriptionGroup?: string): MQSubscription;
}