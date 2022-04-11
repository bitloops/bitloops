import { ISSEMessage, ITopicsMapping, SSE_MESSAGE_TYPE, SubscriptionRouterArgs } from './definitions';
import { IIMDB, IMQ } from '../../services/interfaces';

const PENDING_VALUE = 'PENDING';
const ESTABLISHED_VALUE = 'ESTABLISHED';

class SubscriptionRouter {
	constructor(private imdb: IIMDB, private mq: IMQ) {}

	/**
	 * Listens to published workflow events from the engine
	 */
	async handleNodeIntermediateMessage(data: SubscriptionRouterArgs, subject: string): Promise<void> {
		// console.log('RECEIVED EVENT FROM ENGINE', subject);
		const { payload } = data;
		// console.log('received payload', payload);
		try {
			const { workspaceId, eventName } = this.extractTopicAndWorkspaceFromWildcards(subject);
			console.log(eventName);
			const connectionIds = await this.imdb.getConnectionIdsSubscribedToTopic(workspaceId, eventName);
			const podsConnections = {};
			for (const connectionId of connectionIds) {
				// console.log('connectionId', connectionId);
				const connectionResult = await this.imdb.getConnectionIdValue(connectionId);
				const { state, podId, creds } = connectionResult;
				/** TODO Check if each connection has auth permissions */
				if (state === PENDING_VALUE) continue;
				if (!podsConnections[podId]) podsConnections[podId] = [];
				podsConnections[podId].push(connectionId);
				// console.log('subject', `${podId}.${eventName}`);
			}
			await Promise.all(
				Object.keys(podsConnections).map((podId) =>
					this.mq.publish(`${podId}.${eventName}`, { connections: podsConnections[podId], payload }),
				),
			);
		} catch (error) {
			console.error('error', error);
		}
	}

	/**
	 * Listens to sse server messages
	 * @param data listens for 3 types of tasks [validateConnectionId, addToMapping, store podId]
	 * @param topic ssevents.connectionId - Encapsulates connectionId
	 */
	async registerSubscription(data: ISSEMessage, topic: string) {
		console.log('Received msg from rest with data:', data, 'topic:', topic);
		const { name } = data.type;
		const [, connectionId] = topic.split('.');
		let result;
		console.log(name === SSE_MESSAGE_TYPE.POD_SHUTDOWN);
		try {
			switch (name) {
				case SSE_MESSAGE_TYPE.VALIDATION:
					result = await this.validateConnectionId(connectionId);
					break;
				case SSE_MESSAGE_TYPE.TOPICS_ADD_CONNECTION:
					await this.handleAddTopicsToConnection(data.type, connectionId);
					result = 'Subscriptions written';
					break;
				case SSE_MESSAGE_TYPE.POD_ID_REGISTRATION: {
					const { podId } = data.type;
					await this.imdb.storeConnectionIdValue(connectionId, { state: ESTABLISHED_VALUE, podId });
					await this.imdb.addConnectionToPodId(podId, connectionId);
					result = 'Pod registered';
					break;
				}
				case SSE_MESSAGE_TYPE.TOPIC_UNSUBSCRIBE: {
					const { topic, workspaceId } = data.type;
					// console.log('unsubscribing event', connectionId, workspaceId, topic);
					await this.imdb.handleTopicUnsubscribe(connectionId, workspaceId, topic);
					break;
				}
				case SSE_MESSAGE_TYPE.CONNECTION_END:
					await this.handleConnectionEnd(connectionId);
					break;
				case SSE_MESSAGE_TYPE.POD_SHUTDOWN: {
					const { podId } = data.type;
					console.log('pod shutdown case enetered');
					await this.handlePodShutDown(podId);
					break;
				}
				default:
					console.error('Arbitrary sseEvent type.name');
			}
			this.replyToRequest(data.originalReply, { result, error: null });
		} catch (error) {
			console.error('registerSubscription error', error);
			this.replyToRequest(data.originalReply, { result: null, error: error });
		}
	}

	private async validateConnectionId(connectionId: string): Promise<boolean> {
		const res = await this.imdb.getConnectionIdValue(connectionId);
		console.log('validation res', res);
		if (res !== null) return true;
		else return false;
	}

	private extractTopicAndWorkspaceFromWildcards(subject: string) {
		const tokens = subject.split('.');
		if (tokens.length < 2) throw new Error('Undefined workspaceId');
		const [, workspaceId] = tokens;
		// console.log('extractTopicAndWorkspaceFromWildcards', tokens);
		const eventName = `${tokens[0]}.${tokens.splice(2).join('.')}`;
		// console.log('extractTopicAndWorkspaceFromWildcards-eventName', eventName);

		return { workspaceId, eventName };
	}

	private replyToRequest(originalReply: string, payload: any) {
		if (!originalReply) return;
		this.mq.publish(originalReply, payload);
	}

	private async handleAddTopicsToConnection(msg: ITopicsMapping, connectionId: string) {
		console.log('new connection');
		const { workspaceId, topics } = msg;
		await this.initializeIfNewConnection(msg, connectionId);
		await Promise.all(
			topics.map((topic) => this.imdb.addConnectionIdToTopic({ workspaceId, topic }, connectionId)),
		);
		/** Also store reverse relation, connectionId -> topics */
		await this.imdb.addTopicsToConnectionId(connectionId, { workspaceId, topics });
	}

	// TODO remove connectionId from imdb
	private async handleConnectionEnd(connectionId: string) {
		console.log('connectionId that died', connectionId);
		return this.imdb.removeConnectionId(connectionId);
	}

	// TODO Implement
	private async handlePodShutDown(podId: string) {
		console.log('pod has died', podId);
		return this.imdb.cleanPodState(podId);
	}

	private initializeIfNewConnection(msg: ITopicsMapping, connectionId: string): Promise<void> {
		const { newConnection, creds } = msg;
		if (newConnection === true) {
			return this.imdb.storeConnectionIdValue(connectionId, {
				state: PENDING_VALUE,
				creds: JSON.stringify(creds),
			});
		}
	}
}

export default SubscriptionRouter;
