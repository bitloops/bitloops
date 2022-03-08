import React, { FC } from 'react';

interface GoogleButtonProps {
  loginWithGoogle: () => void;
}

export const GoogleButton: FC<GoogleButtonProps> = (props) => {
  const { loginWithGoogle } = props;
  return (<div style={{
    display: 'flex',
    flexDirection: 'column',
    width: '250px',
    marginLeft: 'calc(50% - 125px)',
  }}>
    <div style={{fontFamily: 'sans-serif', marginBottom: '30px'}}>You need to login to be able to chat.</div>
    <button
      onClick={loginWithGoogle}
      type="button"
      style={{
        position: 'relative',
        background: '#fff',
        borderColor: '#cbd4db',
        color: '#273240',
        fill: '#6f7782',
        fontSize: '16px',
        height: '48px',
        lineHeight: '48px',
        padding: '0 16px',
        cursor: 'pointer',
        alignSelf: 'stretch',
        marginBottom: '32px',
        alignItems: 'center',
        border: '1px solid',
        borderRadius: '2px',
        boxSizing: 'border-box',
        display: 'inline-flex',
        flexShrink: 0,
        justifyContent: 'center',
        overflow: 'hidden',
        transitionDuration: '.2s',
        transitionProperty: 'background,border,box-shadow,color,fill',
        userSelect: 'none',
        // '&:hover': {
        //   backgroundColor: '#f2f3f5',
        // }
      }}
    >
      <svg style={{
        left: '32px',
        position: 'absolute',
        display: 'block',
        overflow: 'hidden',
        flex: '0 0 auto',
        height: '18px',
        width: '18px',
        marginRight: '4px',
      }} viewBox="0 0 18 18">
        <path
          d="M17.64,9.20454545 C17.64,8.56636364 17.5827273,7.95272727 17.4763636,7.36363636 L9,7.36363636 L9,10.845 L13.8436364,10.845 C13.635,11.97 13.0009091,12.9231818 12.0477273,13.5613636 L12.0477273,15.8195455 L14.9563636,15.8195455 C16.6581818,14.2527273 17.64,11.9454545 17.64,9.20454545 L17.64,9.20454545 Z"
          fill="#4285F4"
        />
        <path
          d="M9,18 C11.43,18 13.4672727,17.1940909 14.9563636,15.8195455 L12.0477273,13.5613636 C11.2418182,14.1013636 10.2109091,14.4204545 9,14.4204545 C6.65590909,14.4204545 4.67181818,12.8372727 3.96409091,10.71 L0.957272727,10.71 L0.957272727,13.0418182 C2.43818182,15.9831818 5.48181818,18 9,18 L9,18 Z"
          fill="#34A853"
        />
        <path
          d="M3.96409091,10.71 C3.78409091,10.17 3.68181818,9.59318182 3.68181818,9 C3.68181818,8.40681818 3.78409091,7.83 3.96409091,7.29 L3.96409091,4.95818182 L0.957272727,4.95818182 C0.347727273,6.17318182 0,7.54772727 0,9 C0,10.4522727 0.347727273,11.8268182 0.957272727,13.0418182 L3.96409091,10.71 L3.96409091,10.71 Z"
          fill="#FBBC05"
        />
        <path
          d="M9,3.57954545 C10.3213636,3.57954545 11.5077273,4.03363636 12.4404545,4.92545455 L15.0218182,2.34409091 C13.4631818,0.891818182 11.4259091,0 9,0 C5.48181818,0 2.43818182,2.01681818 0.957272727,4.95818182 L3.96409091,7.29 C4.67181818,5.16272727 6.65590909,3.57954545 9,3.57954545 L9,3.57954545 Z"
          fill="#EA4335"
        />
      </svg>
      Login with Google
    </button>
  </div>);
};

export default GoogleButton;
