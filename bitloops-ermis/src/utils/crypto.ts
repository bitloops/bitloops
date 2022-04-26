import * as crypto from 'crypto';
import { AppOptions } from '../constants';
import { Options } from '../services';

export const getHash = (token: string) => {
   return crypto.createHash(AppOptions.SHA256).update(token+Options.getOption(AppOptions.SHA256_SALT)).digest('hex');
}