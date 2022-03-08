import { v4 as uuid } from 'uuid';
import { MQTopics } from '../../constants';
import { IWorkflow } from '../../services/interfaces';
import { BaseNode, StartNode, ExclusiveNode, TaskNode, CallNode, EndNode, TimerNode } from '../nodes';
import { NodeTypeName, INode } from '../nodes/definitions';
import {
	WorkflowConstructorArgs,
	BitloopsVariables,
	WorkflowParams,
	IBitloopsWorkflowDefinition,
	IToEdge,
	IBitloopsInputConstant,
} from './definitions';
import { IServices } from '../../services/definitions';
import { AuthData } from '../../handlers/bitloopsEngine/definitions';

class Workflow implements IWorkflow {
	private services: IServices;
	private readonly workflowDefinition: IBitloopsWorkflowDefinition;
	private params: WorkflowParams = {};
	private startPayload: Record<string, unknown>;
	private nodes: Record<string, BaseNode>;
	private readonly ENCRYPTION_ALGORITHM = 'aes-256-ctr';

	constructor(workflowConstructorArgs: WorkflowConstructorArgs) {
		const { workflowDefinition, services, payload, originalReply, environmentId, authData } = workflowConstructorArgs;

		this.workflowDefinition = workflowDefinition;
		this.services = services;
		this.startPayload = payload;

		//TODO maybe group together
		const constants = this.parseConstants(environmentId, authData);
		const systemVariables = this.initializeSystemData(originalReply, environmentId);
		this.params = { constants, systemVariables };
		this.nodes = this.getNodes();
	}

	public getId(): string {
		return this.workflowDefinition.id;
	}

	public getWorkflow(): IBitloopsWorkflowDefinition {
		return this.workflowDefinition;
	}

	public getStartPayload(): Record<string, unknown> {
		return this.startPayload;
	}

	public getParams(): WorkflowParams {
		return this.params;
	}

	public getNode(nodeId: string): BaseNode {
		return this.nodes[nodeId];
	}

	public getServices(): IServices {
		return this.services;
	}

	public setParams(workflowParams: WorkflowParams) {
		this.params = workflowParams;
	}

	public setVariablesParams(variables: BitloopsVariables) {
		if (this.params) this.params.variables = variables;
	}

	private initializeSystemData(originalReply: string, environmentId: string): BitloopsVariables {
		const systemVariables: BitloopsVariables = {
			nodes: {},
			workspaceId: this.workflowDefinition.workspaceId,
			environmentId,
		};
		const { Options } = this.services;
		if (this.isRequestReplyMessage(originalReply)) {
			if (systemVariables) {
				systemVariables.originalReply = originalReply;
				systemVariables.originalTopic = Options.getOption(MQTopics.ENGINE_TOPIC);
			}
		}
		return systemVariables;
	}

	private parseConstants(environmentId: string, authData: AuthData): BitloopsVariables {
		const constantsObj = this.workflowDefinition.constants;
		const constants: BitloopsVariables = {};

		if (this.areConstantsSet(constantsObj, environmentId)) {
			const constantsArray = constantsObj[environmentId];
			if (constantsArray) for (const constant of constantsArray) {
				constants[constant.name] = constant.value;
			}
			if(authData) {
				constants.authData = authData;
			}
		}
		constants.startedAt = Date.now();
		constants.instanceId = uuid();
		return constants;
	}

	private areConstantsSet(constantsObj: Record<string, IBitloopsInputConstant[]>, environmentId: string): boolean {
		return constantsObj && environmentId !== null && environmentId !== undefined;
	}

	private isRequestReplyMessage(replyTopic: string): boolean {
		return replyTopic !== undefined;
	}

	public getNodes(): Record<string, BaseNode> {
		const { nodes } = this.workflowDefinition;
		const nodesObj: Record<string, BaseNode> = {};
		for (let i = 0; i < nodes.length; i++) {
			const nodeId = nodes[i].id;
			let node: BaseNode;
			switch (nodes[i].type.name) {
				case NodeTypeName.RequestStartNode:
				case NodeTypeName.MessageStartNode:
				case NodeTypeName.DirectStartNode:
					node = new StartNode(nodeId, this, nodes[i], this.startPayload);
					break;
				case NodeTypeName.TaskNode:
					node = new TaskNode(nodeId, this, nodes[i]);
					break;
				case NodeTypeName.CallNode:
					node = new CallNode(nodeId, this, nodes[i]);
					break;
				case NodeTypeName.ExclusiveNode:
					node = new ExclusiveNode(nodeId, this, nodes[i]);
					break;
				case NodeTypeName.EndNode:
					node = new EndNode(nodeId, this, nodes[i]);
					break;
				case NodeTypeName.TimerIntermediateNode:
					node = new TimerNode(nodeId, this, nodes[i]);
					break;
				default:
					console.error(`${nodes[i].type.name} - Not implemented`);
					console.log(nodes[i]);
					break;
			}
			nodesObj[nodeId] = node;
		}
		return nodesObj;
	}

	public getNodeDefinition(nodeId: string): INode {
		const { nodes } = this.workflowDefinition;
		for (let i = 0; i < nodes.length; i++) {
			if (nodes[i].id === nodeId) {
				return nodes[i];
			}
		}
		console.error('no nodes found!');
	}

	public getNextNode(nodeId: string): BaseNode {
		const currentNode: BaseNode = this.nodes[nodeId];
		const to = this.getToEdges(nodeId);
		return this.nodes[to[0].nodeId];
	}

	public getToEdges(nodeId: string): IToEdge[] | null {
		const { edges } = this.workflowDefinition;
		let to: IToEdge[] | null = null;
		for (let i = 0; i < edges.length; i++) {
			if (edges[i].from === nodeId) {
				to = edges[i].to;
				break;
			}
		}
		return to;
	}

	async handleNode(node: BaseNode): Promise<void> {
		while (node) {
			this.params = await node.execute(this.params);
			node = await node.getNext();
		}
		// await services.imdb.clearServerActiveInstance(Options.getServerUUID(), variables.instanceId);
	}
}

export default Workflow;
