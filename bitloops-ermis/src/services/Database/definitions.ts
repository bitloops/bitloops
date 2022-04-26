import { Document, WithId } from 'mongodb';

export type BasicAuthResponse = {
	workspaceId: string;
	error: Error;
};

export interface IXApiKeyDefinition extends WithId<Document> {
	name?: string;
	description?: string;
	workspaceId: string;
	prefix?: string;
	created_at?: string;
	cached_at?: number;
	status?: number;
}
