import { IXApiKeyDefinition } from "./definitions";

export interface IDatabase {
    connect(): Promise<void>;
    disconnect(): Promise<void>;
    getXApiKey(xApiKey: string): Promise<IXApiKeyDefinition | null>;
}