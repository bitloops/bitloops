import React, { useEffect, useState } from 'react';
import { BitloopsUser, Unsubscribe } from 'bitloops/dist/definitions';

// import './App.css';
import './components/chat.scss';
import bitloops, { DemoChat } from './bitloops';
import Header from './components/Header';
import GoogleButton from './components/GoogleButton';
import Message from './components/Message';

const getInitialMessagesValue = (): DemoChat.Subscription_NewPublicMessage[] => {
  return [];
};

let unsubscribe: Unsubscribe;

function App() {
  const [nickname, setNickname] = useState('');
  const [message, setMessage] = useState('');
  const [user, setUser] = useState<BitloopsUser | null>(null);
  const [messages, setMessages] = useState(() => getInitialMessagesValue());
  
  /**
   * Upon initialization set onAuthStateChange in order
   * to keep track of auth state locally
   */
  React.useEffect(() => {
    bitloops.auth.onAuthStateChange((user: any) => {
      setUser(user);
    });
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    async function subscribe() {
      unsubscribe = await bitloops.subscribe(bitloops.demoChat.Events.NewPublicMessage(), (msg: DemoChat.Subscription_NewPublicMessage) => {
        setMessages(prevState => [...prevState, msg])});
    }
    if (user) {
      setNickname(user.firstName);
      subscribe();
    } else {
      if (unsubscribe) unsubscribe();
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [user]);

  useEffect(() => { console.log('received message', messages)}, [messages]);

  return (
    <>
      {user ? 
        <div className="container clearfix">
          {/* <div className="people-list" id="people-list">
      <div className="search">
        <input type="text" placeholder="search" />
        <i className="fa fa-search"></i>
      </div>
      <ul className="list">
        <li className="clearfix">
          <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_01.jpg" alt="avatar" />
          <div className="about">
            <div className="name">Vincent Porter</div>
            <div className="status">
              <i className="fa fa-circle online"></i> online
            </div>
          </div>
        </li>
        
        <li className="clearfix">
          <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_02.jpg" alt="avatar" />
          <div className="about">
            <div className="name">Aiden Chavez</div>
            <div className="status">
              <i className="fa fa-circle offline"></i> left 7 mins ago
            </div>
          </div>
        </li>
        
        <li className="clearfix">
          <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_03.jpg" alt="avatar" />
          <div className="about">
            <div className="name">Mike Thomas</div>
            <div className="status">
              <i className="fa fa-circle online"></i> online
            </div>
          </div>
        </li>
        
        <li className="clearfix">
          <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_04.jpg" alt="avatar" />
          <div className="about">
            <div className="name">Erica Hughes</div>
            <div className="status">
              <i className="fa fa-circle online"></i> online
            </div>
          </div>
        </li>
        
        <li className="clearfix">
          <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_05.jpg" alt="avatar" />
          <div className="about">
            <div className="name">Ginger Johnston</div>
            <div className="status">
              <i className="fa fa-circle online"></i> online
            </div>
          </div>
        </li>
        
        <li className="clearfix">
          <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_06.jpg" alt="avatar" />
          <div className="about">
            <div className="name">Tracy Carpenter</div>
            <div className="status">
              <i className="fa fa-circle offline"></i> left 30 mins ago
            </div>
          </div>
        </li>
        
        <li className="clearfix">
          <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_07.jpg" alt="avatar" />
          <div className="about">
            <div className="name">Christian Kelly</div>
            <div className="status">
              <i className="fa fa-circle offline"></i> left 10 hours ago
            </div>
          </div>
        </li>
        
        <li className="clearfix">
          <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_08.jpg" alt="avatar" />
          <div className="about">
            <div className="name">Monica Ward</div>
            <div className="status">
              <i className="fa fa-circle online"></i> online
            </div>
          </div>
        </li>
        
        <li className="clearfix">
          <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_09.jpg" alt="avatar" />
          <div className="about">
            <div className="name">Dean Henry</div>
            <div className="status">
              <i className="fa fa-circle offline"></i> offline since Oct 28
            </div>
          </div>
        </li>
        
        <li className="clearfix">
          <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_10.jpg" alt="avatar" />
          <div className="about">
            <div className="name">Peyton Mckinney</div>
            <div className="status">
              <i className="fa fa-circle online"></i> online
            </div>
          </div>
        </li>
      </ul>
    </div> */}
          <div><Header user={user} logout={() => bitloops.app.auth.clearAuthentication()}/></div>
          {/* <span>Nickname:</span>
          <input 
            type="text"
            value={nickname}
            className="nickname-input"
            onChange={(event) => { setNickname(event.target.value)}}
            onBlur={(event) => { setNickname(event.target.value)}}
          /> */}
          <div className="chat">
            {/* <div className="chat-header clearfix">
              <img src="https://s3-us-west-2.amazonaws.com/s.cdpn.io/195612/chat_avatar_01_green.jpg" alt="avatar" />
        
              <div className="chat-about">
                <div className="chat-with">Chat with Vincent Porter</div>
                <div className="chat-num-messages">already 1 902 messages</div>
              </div>
              <i className="fa fa-star"></i>
            </div>  */}
            {/* <!-- end chat-header --> */}
            <div className="chat-history">
              <ul style={{listStyle: 'none'}}>{messages && messages.map(message => (
              <Message 
                key={message.sendAt+message.senderUid}
                isMine={message.senderUid === user.uid}
                sendAt={message.sendAt}
                senderNickname={message.senderNickname}
                message={message.message}
              />
             ))}</ul></div>
            <div className="chat-message clearfix">
              <input 
                value={message}
                name="message-to-send"
                id="message-to-send"
                onChange={(event) => {setMessage(event.target.value)}}
                onBlur={(event) => { setMessage(event.target.value)}}
                onKeyPress={(event) => {
                  if (event.key === 'Enter') {
                    console.log('sending', {message, nickname, senderUid: user.uid});
                    bitloops.demoChat.publicMessageSent({message, nickname, senderUid: user.uid});
                    setMessage('');
                  } 
                }}
                placeholder="Type your message"
              />           
              <button
                onKeyPress={() => {
                    console.log('sending', {message, nickname, senderUid: user.uid});
                    bitloops.demoChat.publicMessageSent({message, nickname, senderUid: user.uid});
                    setMessage('');
                }}
                onClick={() => {
                  console.log('sending', {message, nickname, senderUid: user.uid});
                  bitloops.demoChat.publicMessageSent({message, nickname, senderUid: user.uid});
                  setMessage('');
              }}
              >Send</button>
            </div>
          </div>
        </div> :
        <div>
          {!user && <GoogleButton loginWithGoogle={() => bitloops.demoChat.bitloopsApp.auth.authenticateWithGoogle()}/>}
        </div>
      }
    </>
  );
}

export default App;
