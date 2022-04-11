import { TSseConnectionIds } from '../../routes/definitions';

export default class SubscriptionEvents {
	constructor(private sseConnectionIds: TSseConnectionIds) {}

	public handle(data: any, subject: string) {
		// console.log('data received', data);
		const { eventName } = this.extractInfoFromSubject(subject);
		this.notifySubscribedConnections(eventName, data.payload, data.connections);
	}

	private notifySubscribedConnections(eventName: string, data: any, connections: string[]) {
		// console.log('topicConnections about to notify', connections);
		for (const connectionId of connections) {
			const connection = this.sseConnectionIds[connectionId];
			// console.log('val is PENDING', connection === PENDING_VALUE);
			if (!connection) {
				console.error('Received unexpected connection from sr');
				continue;
			}
			connection.write(`event: ${eventName}\n`);
			connection.write(`data: ${JSON.stringify(data)}\n\n`);
		}
	}

	private extractInfoFromSubject(subject: string) {
		const tokens = subject.split('.');
		// Remove workspaceId and workflow-events prefix
		const eventName = tokens.slice(2).join('.');
		console.log('tokens', tokens);
		return { eventName };
	}
}
