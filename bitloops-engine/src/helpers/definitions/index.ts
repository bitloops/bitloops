import { BitloopsVariables } from '../../entities/workflow/definitions';
import { WorkflowContext } from '../../handlers/bitloopsEngine/definitions';

export type ReplaceVarsParams = {
	payload?: BitloopsVariables;
	secrets?: BitloopsVariables;
	variables?: BitloopsVariables;
	systemVariables?: BitloopsVariables;
	constants?: BitloopsVariables;
	output?: BitloopsVariables;
	context: WorkflowContext;
};
