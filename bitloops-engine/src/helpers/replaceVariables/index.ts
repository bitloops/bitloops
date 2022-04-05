import * as crypto from 'crypto';
import { v4 } from 'uuid';
import { ENCRYPTION_KEY } from '../../constants';
import { ITypedVariable, WorkspaceSecretsInfo } from '../../entities/workflow/definitions';
import Services, { Options } from '../../services';
import { ReplaceVarsParams } from '../definitions';
import { dataToBuffer as dataToBufferImport, bufferToData as bufferToDataImport } from '../../util/messagePack';

const ENCRYPTION_KEY_DELIMETER = ':';

const secretKey = Options.getOption(ENCRYPTION_KEY);
const algorithm = 'aes-256-ctr';

const sanitizeEvalValue = (evalValue: string): string => {
	return evalValue?.replace('Workflow.bitloopsSecretEncryptionKey', '"Nice try!"');
};

const decrypt = (hash: string, key: string): string => {
	const iv = hash.split(ENCRYPTION_KEY_DELIMETER)[0];
	const content = hash.split(ENCRYPTION_KEY_DELIMETER)[1];
	const decipher = crypto.createDecipheriv(algorithm, key, Buffer.from(iv, 'hex'));
	const decrypted = Buffer.concat([decipher.update(Buffer.from(content, 'hex')), decipher.final()]);
	return decrypted.toString();
};

// DO NOT DELETE. This function is used by eval.
const decryptByWorkspaceId = (hash: string, workspaceId: string): string => {
	const encryptionKey = crypto.createHmac('sha256', secretKey).update(workspaceId).digest('hex').substr(0, 32);
	return decrypt(hash, encryptionKey);
};

// DO NOT DELETE. This function is used by eval.
const encryptByWorkspaceId = (plainText: string, workspaceId: string): string => {
	const encryptionKey = crypto.createHmac('sha256', secretKey).update(workspaceId).digest('hex').substr(0, 32);
	const iv = crypto.randomBytes(16);
	const cipher = crypto.createCipheriv('aes-256-ctr', encryptionKey, iv);
	const encrypted = Buffer.concat([cipher.update(plainText), cipher.final()]);
	const hexIv = iv.toString('hex');
	const content = encrypted.toString('hex');
	return hexIv + ':' + content;
};

// DO NOT DELETE. This function is used by eval.
const hashToken = (token) => {
	return crypto
		.createHash('sha256')
		.update(token + secretKey)
		.digest('hex');
};

export const replaceVars = async (vars: ITypedVariable[], params: ReplaceVarsParams): Promise<Record<string, any>> => {
	const { variables, output, systemVariables, constants, payload, context } = params;
	const evaluatedParams = {};
	const dataToBuffer = dataToBufferImport;
	const bufferToData = bufferToDataImport;
	const uuid = v4;
	// console.log('OUTPUT IS', output);
	if (vars === undefined || vars === null) return evaluatedParams;
	const secretIds = getSecretIds(vars);
	const encryptedSecrets = await getWorkspaceSpecifiedSecrets(
		systemVariables.workspaceId,
		secretIds,
		systemVariables.environmentId,
	);

	for (let i = 0; i < vars.length; i++) {
		const secrets = {};
		if (vars[i].evalValue?.includes('secrets["')) {
			const key = crypto
				.createHmac('sha256', secretKey)
				.update(systemVariables.workspaceId)
				.digest('hex')
				.substr(0, 32);
			console.log('key', key);
			for (const encryptedSecret in encryptedSecrets) {
				secrets[encryptedSecret] = decrypt(encryptedSecrets[encryptedSecret], key);
			}
		}
		const sanitizedEvalValue = sanitizeEvalValue(vars[i].evalValue);
		//TODO check if we need parenthesis
		try {
			evaluatedParams[vars[i].name] = eval('(' + sanitizedEvalValue + ')');
		} catch (error) {
			console.error('eval failed', error);
			// this will be removed
			forceValueToBeEvaluatedAsString(evaluatedParams, vars[i].name, sanitizedEvalValue);
		}
	}
	return evaluatedParams;
};

const forceValueToBeEvaluatedAsString = (evalParams: any, varName: string, evalValue) => {
	try {
		evalParams[varName] = eval('("' + evalValue + '")');
	} catch (error) {
		console.error('forced eval to string failed', error);
	}
};

const getSecretIds = (vars: ITypedVariable[]) => {
	const secretIds = [];
	for (let i = 0; i < vars.length; i++) {
		if (vars[i].evalValue?.includes('secrets["')) {
			const splitByBraces = vars[i].evalValue?.split('secrets["')[1];
			const secretId = splitByBraces.trim().split('"]')[0];
			secretIds.push(secretId);
		}
	}
	return secretIds;
};

const getWorkspaceSpecifiedSecrets = async (
	workspaceId: string,
	secretIds: string[],
	environmentId: string,
): Promise<Record<string, string>> => {
	const { db, workspaceSecretsCache } = Services.getServices();

	// console.log('All secretIds needed:', secretIds);
	const cachedSecrets: Record<string, WorkspaceSecretsInfo> = await workspaceSecretsCache.fetchSecrets(
		workspaceId,
		secretIds,
	);
	const cachedSecretsIds = Object.keys(cachedSecrets);
	const missingSecretsIds = [...secretIds].filter((id) => !cachedSecretsIds.includes(id));
	const secretsToBeFetched = missingSecretsIds.length > 0 ? missingSecretsIds : [...secretIds];

	let secrets = cachedSecrets;
	if (!cachedSecrets || missingSecretsIds.length > 0) {
		// console.log('Some(or all) secrets missing from cache, fetching from DB, cachedSecrets =', cachedSecrets);
		const notCachedSecrets: Record<string, WorkspaceSecretsInfo> = await db.getSecretsById(
			workspaceId,
			secretsToBeFetched,
		);
		// console.log('notCachedSecrets', notCachedSecrets);
		await workspaceSecretsCache.cacheSecrets(workspaceId, notCachedSecrets);
		secrets = { ...secrets, ...notCachedSecrets };
	}

	const specifiedSecrets = {};
	const ids = Object.keys(secrets);
	for (let i = 0; i < ids.length; i++) {
		const secretId = ids[i];
		const secret = secrets[secretId].environments[environmentId];
		specifiedSecrets[secretId] = secret.secretValue;
	}
	// console.log('secrets', secrets)
	return specifiedSecrets;
};
