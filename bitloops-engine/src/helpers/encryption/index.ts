import * as crypto from 'crypto';
import { Options } from '../../services';

const secretKey = Options.getOption('ENCRYPTION_KEY');
const algorithm = 'aes-256-ctr';

export const decrypt = (hash, key) => {
	const iv = hash.split(':')[0];
	const content = hash.split(':')[1];
	const decipher = crypto.createDecipheriv(algorithm, key, Buffer.from(iv, 'hex'));
	const decrypted = Buffer.concat([decipher.update(Buffer.from(content, 'hex')), decipher.final()]);
	return decrypted.toString();
};
