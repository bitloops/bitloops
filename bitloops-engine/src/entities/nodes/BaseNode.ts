import { MQTopics, ADMIN_WORKSPACE_ID } from '../../constants';
import { replaceVars } from '../../helpers/replaceVariables';
import { IServices } from '../../services/definitions';
import { IWorkflow } from '../../services/interfaces';
import { ITypedVariable, variableTypes, WorkflowParams } from '../workflow/definitions';
import {
	AdminType,
	EventType,
	INode,
	IntermediateMessageKeys,
	NodeLoggerData,
	NodeMessageType,
	PublishEventType,
	SendEventParams,
} from './definitions';

export abstract class BaseNode {
	protected id: string;
	protected workflow: IWorkflow;
	protected services: IServices;
	protected startedAt: number;
	protected nodeDefinition: INode;

	public constructor(id: string, workflow: IWorkflow, nodeDefinition: INode) {
		this.id = id;
		this.services = workflow.getServices();
		this.workflow = workflow;
		this.nodeDefinition = nodeDefinition;
	}

	async getNext(): Promise<BaseNode> {
		const currentNode: BaseNode = this.workflow.getNode(this.id);
		const to = this.workflow.getToEdges(this.id);
		return this.workflow.getNode(to[0].nodeId);
	}

	getId(): string {
		return this.id;
	}

	public async execute(workflowParams: WorkflowParams): Promise<WorkflowParams> {
		this.log(workflowParams);

		const { messages } = this.nodeDefinition;
		if (messages) {
			await this.handleIntermediateEvents(workflowParams, messages);
		}

		return workflowParams;
	}

	private async handleIntermediateEvents(workflowParams: Readonly<WorkflowParams>, messages: NodeMessageType) {
		const { workspaceId } = workflowParams.systemVariables;
		const { Options } = this.services;
		for (const [messageKey, messagesArray] of Object.entries(messages)) {
			if (messageKey === IntermediateMessageKeys.ADMIN) {
				await this.sendAdminEvents({ workspaceId, messages: messagesArray as AdminType[], workflowParams, Options });
			} else if (messageKey === IntermediateMessageKeys.EVENT) {
				await this.sendEngineEvents({ workspaceId, messages: messagesArray as EventType[], workflowParams, Options });
			} else if (messageKey === IntermediateMessageKeys.PUBLISH_EVENT) {
				await this.sendPublishEvents({ workspaceId, messages: messagesArray as PublishEventType[], workflowParams });
			} else {
				throw new Error('Message type not implemented!');
			}
		}
	}

	private async sendPublishEvents(publishEventsParams: SendEventParams) {
		const { workspaceId, messages, workflowParams } = publishEventsParams;
		for (let i = 0; i < messages.length; i++) {
			const publishMessage = messages[i] as PublishEventType;
			const payloadDefinition = publishMessage.payload;
			const prefix = publishMessage.type;
			const topicDefinition: ITypedVariable[] = [
				{
					type: variableTypes.string,
					name: "name",
					evalValue: publishMessage.topic,
				}
			]
			const evalTopic = await replaceVars(topicDefinition, workflowParams);
			// TODO add version to beginning
			const topic = `${prefix}.${workspaceId}.${evalTopic.name}`;
			const payload = await replaceVars(payloadDefinition, workflowParams);
			const publishParams = {
				payload
			};
			this.services.mq.publish(topic, publishParams);
		}
	}

	private async sendEngineEvents(engineEventsParams: SendEventParams) {
		const { workspaceId, messages, workflowParams, Options } = engineEventsParams;
		for (let i = 0; i < messages.length; i++) {
			const engineMessage = messages[i] as EventType;
			const payloadDefinition = engineMessage.payload;
			const topic = Options.getOption(MQTopics.ENGINE_EVENTS_TOPIC);
			const messageId = engineMessage.messageId;
			const payload = await replaceVars(payloadDefinition, workflowParams);
			const publishParams = { messageId, workspaceId, payload };
			this.services.mq.publish(topic, publishParams);
		}
	}

	private async sendAdminEvents(adminEventsParams: SendEventParams) {
		const { workspaceId, messages, workflowParams, Options } = adminEventsParams;
		const adminWorkspaceId = Options.getOption(ADMIN_WORKSPACE_ID);
		if (workspaceId !== adminWorkspaceId) throw new Error('Unauthorized:)');
		for (let i = 0; i < messages.length; i++) {
			const adminMessage = messages[i] as AdminType;
			const { evalTopic, command } = adminMessage;
			const topicDefinition: ITypedVariable[] = [
				{
					type: variableTypes.string,
					name: "name",
					evalValue: evalTopic,
				}
			]
			const topic = await replaceVars(topicDefinition, workflowParams);
			const topicName = topic.name;
			const payloadDefinition = adminMessage.payload;
			const payload = await replaceVars(payloadDefinition, workflowParams);
			const publishParams = {
				command,
				payload
			}
			this.services.mq.publish(topicName, publishParams);
		}
	}

	private log(workflowParams: WorkflowParams) {
		const now = Date.now();
		const { variables, constants } = workflowParams;
		const { id: workflowId, workspaceId, debugId: workflowDebugId } = this.workflow.getWorkflow();
		const logger = this.services.logger;
		const loggerData: NodeLoggerData = {
			workspaceId,
			workflowId,
			instanceId: constants.instanceId,
			nodeId: this.id,
			duration: now - this.startedAt,
			debugId: this.nodeDefinition.debugId ?? workflowDebugId,
			occurredAt: now,
			durationSinceStart: now - constants.startedAt,
			debugInfo: JSON.stringify({ variables }),
		};
		logger.log(loggerData);
	}
}

export default BaseNode;
