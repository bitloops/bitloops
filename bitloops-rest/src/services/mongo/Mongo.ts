import { FirebaseCredentials } from './../definitions/index';
import { MongoClient } from 'mongodb';
import { Options } from '..';
import { IXApiKeyDefinition } from '../definitions';
import { IDatabase } from '../interfaces';
import { KeycloakSettings } from '../../constants';

// Connection URL
const baseUrl = 'mongodb://localhost:27017';

// Database Name
const DB_NAME = 'bitloops_managed';
const MONGO_ID_DELIMITER = ':';

class Mongo implements IDatabase {
	private client: MongoClient;

	constructor() {
		const urlPrefix = Options.getOption('MONGO_URL_PREFIX');
		const urlSuffix = Options.getOption('MONGO_URL_SUFFIX');
		const user = Options.getOption('MONGO_USER');
		const pass = Options.getOption('MONGO_PASSWORD');

		let url: string;
		if (urlPrefix && urlSuffix) {
			url = `${urlPrefix}${urlSuffix}`;
			if (user && pass) url = `${urlPrefix}${user}:${pass}${urlSuffix}`;
		} else url = baseUrl;
		// console.log(url);
		const client = new MongoClient(url);
		this.client = client;
	}
	async connect() {
		// Use connect method to connect to the server
		console.info('Connecting to Mongo...');
		await this.client.connect();
		console.info('Connected successfully to MongoDB server');
	}

	async disconnect() {
		await this.client.close();
		console.info('Disconnected successfully from MongoDB server');
	}

	async getWorkflowsTriggeredByEvent(
		workspaceId: string,
		messageId: string,
	): Promise<Array<{ workflowId: string; version?: number }> | null> {
		const collectionName = 'workflow_event_triggers';
		const mongoId = `${workspaceId}${MONGO_ID_DELIMITER}${messageId}`;

		const collection = this.client.db(DB_NAME).collection(collectionName);
		const findResult = await collection.findOne({ _id: mongoId });
		if (findResult === null) {
			return null;
		}
		return findResult.workflows;
	}

	async getXApiKey(xApiKey: string): Promise<IXApiKeyDefinition | null> {
		const collectionName = 'x_api_keys';
		const mongoId = xApiKey;

		const collection = this.client.db(DB_NAME).collection(collectionName);
		const findResult = (await collection.findOne({ _id: mongoId })) as IXApiKeyDefinition;
		return findResult ?? null;
	}

	async getFirebaseCredentials(providerId: string): Promise<FirebaseCredentials | null> {
		console.log(1);
		const collectionName = 'firebase_credentials';
		const mongoId = providerId;

		const collection = this.client.db(DB_NAME).collection(collectionName);
		const findResult = (await collection.findOne({ _id: mongoId })) as FirebaseCredentials;
		return findResult ?? null;
	}

	async getSecrets(workflowId: string, version: number): Promise<any> {
		const collectionName = 'secrets';
		const mongoId = `${workflowId}${MONGO_ID_DELIMITER}${version}`;

		const collection = this.client.db(DB_NAME).collection(collectionName);
		const findResult = await collection.findOne({ _id: mongoId });
		return findResult;
	}

	async getProviderClientSecret(providerId: string, clientId: string): Promise<string> {
		const collectionName = 'keycloak_providers';
		const collection = this.client.db(DB_NAME).collection(collectionName);
		const findResult = await collection.findOne(
			{ name: providerId },
			{
				projection: {
					clients: { $elemMatch: { id: clientId } },
					_id: 0,
				},
			},
		);
		console.log('client secret found', findResult);
		const secret = findResult?.clients?.[0].secret;
		return secret ?? null;
	}
}

export default Mongo;
