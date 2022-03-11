import { unpack, pack } from 'msgpackr';

export const dataToBuffer = (data: any): Buffer => pack(data);
export const bufferToData = (data: Buffer): any => unpack(data);
