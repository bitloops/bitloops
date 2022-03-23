import React from 'react';
import './chat.scss';

interface MessageProps {
  senderNickname: string;
  message: string;
  sentAt?: number;
  isMine: boolean;
}

export const Message: React.FC<MessageProps> = (props: MessageProps) => {
  const { senderNickname, message, isMine, sentAt } = props;

   return !isMine ?
    (<li>
      <div>{test}</div>
      <div className="message-data">
        <span className="message-data-name"><i className="fa fa-circle online"></i>{senderNickname}</span>
        {sentAt && <span className="message-data-time">{Intl.DateTimeFormat(navigator.language, { hour: "numeric", minute: "numeric", second: 'numeric' }).format(sentAt)}</span> }
      </div>
      <div className="message my-message">{message}</div>
    </li>) :
    (<li className="clearfix">
      <div className="message-data align-right">
      {sentAt && <span className="message-data-time" >{Intl.DateTimeFormat(navigator.language, { hour: "numeric", minute: "numeric", second: 'numeric' }).format(sentAt)}</span>} &nbsp; &nbsp;
        <span className="message-data-name" >{senderNickname}</span> <i className="fa fa-circle me"></i>
      </div>
      <div className="message other-message float-right">{message}</div>
    </li>);
};

export default Message;
