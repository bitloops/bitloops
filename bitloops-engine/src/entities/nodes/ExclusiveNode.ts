import { BaseNode } from './BaseNode';
import { IExclusiveNode } from './definitions';
import { BitloopsVariables, ITypedVariable, WorkflowParams } from '../workflow/definitions';
import { replaceVars } from '../../helpers/replaceVariables';

class ExclusiveNode extends BaseNode {
	async execute(workflowParams: WorkflowParams): Promise<WorkflowParams> {
		// console.log('EXCLUSIVE NODE ID-->', this.id);
		this.startedAt = Date.now();
		return super.execute(workflowParams);
	}

	async getNext(): Promise<BaseNode> {
		const nodeDefinition = this.nodeDefinition as IExclusiveNode;
		const workflowParams = this.workflow.getParams();
		const { variables, constants } = workflowParams;
		const expressionValue = eval(nodeDefinition.type.parameters.expression);
		const cases = nodeDefinition.type.parameters.cases;
		const toEdges = this.workflow.getToEdges(this.id);
		console.log('expressionValue', expressionValue);
		for (const myCase of cases) {
			if (expressionValue === eval(myCase.evalValue)) {
				console.log('found match with', myCase.evalValue);
				const nodeId = toEdges[myCase.index].nodeId;
				const outputs = myCase.output;
				const output = await replaceVars(outputs, workflowParams);
				workflowParams.variables = { ...workflowParams.variables, ...output };
				// this.replaceOutputs(outputs, variables, constants);
				return this.workflow.getNode(nodeId);
			}
		}
		const defaultCaseIndex = nodeDefinition.type.parameters.default.index ?? 0;
		const nodeId = toEdges[defaultCaseIndex].nodeId; // 0 is the default
		const outputs = nodeDefinition.type.parameters.default.output;
		const output = await replaceVars(outputs, workflowParams);
		workflowParams.variables = { ...workflowParams.variables, ...output };
		// this.replaceOutputs(outputs, variables, constants);
		return this.workflow.getNode(nodeId);
	}

	// constants is used in eval DO NOT delete
	private replaceOutputs(
		outputs: ITypedVariable[],
		variables: BitloopsVariables,
		constants: BitloopsVariables,
	): void {
		if (outputs === undefined || outputs === null) return;
		for (const output of outputs) {
			variables[output.name] = eval(output.evalValue);
		}
		this.workflow.setVariablesParams(variables);
	}
}

export default ExclusiveNode;
