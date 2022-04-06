import {
	IRequiredTypedVariable,
	BitloopsVariables,
	IBitloopsWorkflowDefinition,
	IBitloopsInputConstant,
} from './../workflow/definitions/index';
import { replaceVars } from './../../helpers/replaceVariables/index';
import { BaseNode } from './BaseNode';
import { ICallNode, INode, IStartNodeType } from './definitions';
import { WorkflowParams } from '../workflow/definitions';
import Workflow from '../workflow/Workflow';
import { WorkflowDefinition } from '../workflow/WorkflowDefinition';

class CallNode extends BaseNode {
	async execute(workflowParams: WorkflowParams): Promise<WorkflowParams> {
		console.log('CALL NODE ID-->', this.id);
		const { systemVariables } = workflowParams;
		const executed = systemVariables?.nodes[this.id]?.executed;
		if (!executed) {
			await this.executeSubWorkflow(workflowParams);
			return null;
		}
		const nodeDefinition = this.nodeDefinition as ICallNode;
		const replaceVarsParams = { output: systemVariables?.nodes[this.id].output, ...workflowParams };
		const output = await replaceVars(nodeDefinition.type.output, replaceVarsParams);
		workflowParams.variables = { ...workflowParams.variables, ...output };
		return super.execute(workflowParams);
	}

	async getNext() {
		const executed = this.workflow.getParams()?.systemVariables?.nodes[this.id]?.executed;
		if (!executed) {
			return null;
		}
		return super.getNext();
	}

	private getStartNodeDefinition(blsWorkflowDefinition: IBitloopsWorkflowDefinition, nodeId: string): INode {
		const { nodes } = blsWorkflowDefinition;
		for (let i = 0; i < nodes.length; i++) {
			if (nodes[i].id === nodeId) return nodes[i];
		}
	}

	private async executeSubWorkflow(workflowParams: WorkflowParams): Promise<void> {
		const nodeDefinition = this.nodeDefinition as ICallNode;
		const { workflowId, nodeId, workflowVersion, input } = nodeDefinition.type.parameters;
		// TODO check subWorkflow environmentId === parent's ?
		const { environmentId } = workflowParams.systemVariables;
		const subWorkflowMainInfo = { workflowId, workflowVersion, environmentId };
		const subWorkflowDefinition = await WorkflowDefinition.get(subWorkflowMainInfo);

		const payload = await replaceVars(input, workflowParams);
		const subworkflowStartNodeDefinition = this.getStartNodeDefinition(subWorkflowDefinition, nodeId);
		const { input: subWorkflowInput } = subworkflowStartNodeDefinition.type as IStartNodeType;

		const constants = this.initializeConstants(subWorkflowDefinition.constants, environmentId); // fix/constants are init twice
		const variables = this.initializeSubWorkflowVariables(subWorkflowInput, payload, constants);

		/** Constructor writes constants and (its) systemVariables */
		// TODO create SubWorkflow class that will extend Workflow and override some of its functions
		const subWorkflow = new Workflow({
			workflowDefinition: subWorkflowDefinition,
			services: this.services,
			environmentId,
			context: workflowParams.context,
		});
		const { workspaceId } = subWorkflow.getWorkflow();
		const {
			id: parentWorkflowId,
			workspaceId: parentWorkspaceId,
			version: parentWorkflowVersion,
		} = this.workflow.getWorkflow();
		const systemVariables = this.initializeSubWorkflowSystem(
			{ parentWorkspaceId, parentWorkflowId, parentWorkflowVersion, parentEnvironmentId: environmentId },
			workflowParams,
			workspaceId,
			environmentId,
		);
		/** Trigger would write variables and secrets */
		// TODO improve overwriting of constants/system from setParams
		subWorkflow.setParams({ variables, systemVariables, constants, context: workflowParams.context });
		// TODO get node shown by trigger to be safe
		const startingNode = await subWorkflow.getNode(subworkflowStartNodeDefinition.id).getNext();
		subWorkflow.handleNode(startingNode);
	}

	/** payload is used in eval */
	private initializeSubWorkflowVariables(
		input: IRequiredTypedVariable[],
		payload: Record<string, any>,
		constants: BitloopsVariables): BitloopsVariables {
		const variables: any = {};
		for (const element of input) {
			variables[element.name] = eval(element.evalValue);
		}
		return variables;
	}

	private initializeSubWorkflowSystem(
		parentWorkflowIdParams,
		workflowParams: WorkflowParams,
		workspaceId: string,
		environmentId: string,
	) {
		const { parentWorkflowId, parentWorkspaceId, parentWorkflowVersion, parentEnvironmentId } =
			parentWorkflowIdParams;
		const systemVariables = {
			nodes: {},
			workspaceId,
			environmentId,
			parent: {
				id: this.id,
				workflowId: parentWorkflowId,
				workspaceId: parentWorkspaceId,
				environmentId: parentEnvironmentId,
				workflowVersion: parentWorkflowVersion,
				workflowParams,
			},
		};
		return systemVariables;
	}

	private initializeConstants(
		constantsObj: Record<string, IBitloopsInputConstant[]>,
		environmentId: string,
	): BitloopsVariables {
		const constants: BitloopsVariables = {};
		if (this.areConstantsSet(constantsObj, environmentId)) {
			const constantsArray = constantsObj[environmentId];
			if (constantsArray)
				for (const constant of constantsArray) {
					constants[constant.name] = constant.value;
				}
		}
		// constants.startedAt = Date.now();
		// constants.instanceId = uuid();
		return constants;
	}

	private areConstantsSet(constantsObj: Record<string, IBitloopsInputConstant[]>, environmentId: string): boolean {
		return constantsObj && environmentId !== null && environmentId !== undefined;
	}
}

export default CallNode;
