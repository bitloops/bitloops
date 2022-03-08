import { BaseNode } from './BaseNode';
import { replaceVars } from '../../helpers/replaceVariables';
import { WorkflowParams } from '../workflow/definitions';
import { IEndNode } from './definitions';
import { WorkflowDefinition } from '../workflow/WorkflowDefinition';
import Workflow from '../workflow/Workflow';

class EndNode extends BaseNode {
	async execute(workflowParams: WorkflowParams): Promise<WorkflowParams> {
		this.startedAt = Date.now();
		console.log('END NODE ID-->', this.id);
		const { constants, variables, systemVariables } = workflowParams;
		const { mq, logger } = this.services;
		const node = this.nodeDefinition as IEndNode;
		if (systemVariables.originalReply) {
			const replaceVarsParams = { constants, variables, systemVariables };
			const message = await replaceVars(node.type.output, replaceVarsParams);
			mq.publish(systemVariables.originalReply, message);
		}
		return super.execute(workflowParams);
	}

	async getNext() {
		const systemVariables = this.workflow.getParams()?.systemVariables;
		if (systemVariables.parent) {
			await this.continueParentWorkflow(systemVariables.parent);
		}
		return null;
	}

	private async continueParentWorkflow(parentState) {
		// console.log('current workflow has parentState-->', parentState);
		const { systemVariables: parentSystemVariables } = parentState.workflowParams;
		const parentId = parentState.id;
		if (!parentSystemVariables.nodes[parentId]) parentSystemVariables.nodes[parentId] = {};
		parentSystemVariables.nodes[parentId].executed = true;

		// TODO add (sub)workflow output parsing in execute instead?
		const nodeDefinition = this.nodeDefinition as IEndNode;
		if (nodeDefinition.type.output.length >= 0) {
			const replaceVarsParams = this.workflow.getParams();
			const workflowOutput = await replaceVars(nodeDefinition.type.output, replaceVarsParams);
			parentSystemVariables.nodes[parentId].output = workflowOutput;
		}

		const { workflowId, workflowVersion } = parentState;
		// TODO make all getNext async and convert to await
		const parentWorkflowMainInfo = {
			workflowId,
			workflowVersion,
			environmentId: parentSystemVariables.environmentId,
		};
		WorkflowDefinition.get(parentWorkflowMainInfo).then((parentWorkflowDefinition) => {
			const parentWorkflow = new Workflow({
				workflowDefinition: parentWorkflowDefinition,
				services: this.services,
				environmentId: parentSystemVariables.environmentId,
			});
			parentWorkflow.setParams(parentState.workflowParams);
			const startingNode = parentWorkflow.getNode(parentId);
			parentWorkflow.handleNode(startingNode);
		});
	}
}

export default EndNode;
