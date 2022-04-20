import admin from 'firebase-admin';
import { Document, WithId } from 'mongodb';
import {
	IFirebaseConnectionsCache,
	IRunningRequestsCache,
	ISecretCache,
	IXApiKeyCache,
	IWorkflowEventTriggerCache,
	IDatabase,
	IMQ,
	ILogger,
	IFirebaseTokensCache,
	IIMDB,
} from '../interfaces';

export type Services = {
	firebaseConnectionsCache?: IFirebaseConnectionsCache;
	firebaseTokensCache?: IFirebaseTokensCache;
	runningRequestsCache?: IRunningRequestsCache;
	secretCache?: ISecretCache;
	xApiKeyCache?: IXApiKeyCache;
	workflowEventTriggerCache?: IWorkflowEventTriggerCache;
	db?: IDatabase;
	mq?: IMQ;
	imdb?: IIMDB;
	logger?: ILogger;
	Options?: any; // TODO make Options singleton?;
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

export interface FirebaseCredentials extends WithId<Document> {
	providerId?: string;
	credentials: admin.ServiceAccount;
	workspaceId: string;
	created_at?: string;
	cached_at?: number;
}

export interface BasicAuthResponse {
	_id?: string;
	name?: string;
	description?: string;
	workspaceId: string;
	created_at?: string;
	cached_at?: number;
	status?: number;
}

export type tokenInfo = {
	valid: boolean;
	cached_at: number;
	decoded_token?: admin.auth.DecodedIdToken;
};
