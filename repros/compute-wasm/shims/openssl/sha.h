// WASI shim for OpenSSL SHA-1 (Compute Platform/sha.h). Real SHA-1, header-only.
#pragma once
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#define SHA_DIGEST_LENGTH 20
typedef struct { uint32_t h[5]; uint64_t len; unsigned char buf[64]; uint32_t blen; } SHA_CTX;
static inline uint32_t _s1rol(uint32_t v,int b){return (v<<b)|(v>>(32-b));}
static inline void _s1blk(SHA_CTX*c,const unsigned char*p){
  uint32_t w[80];int i;
  for(i=0;i<16;i++)w[i]=((uint32_t)p[i*4]<<24)|((uint32_t)p[i*4+1]<<16)|((uint32_t)p[i*4+2]<<8)|p[i*4+3];
  for(i=16;i<80;i++)w[i]=_s1rol(w[i-3]^w[i-8]^w[i-14]^w[i-16],1);
  uint32_t a=c->h[0],b=c->h[1],d=c->h[2],e=c->h[3],f=c->h[4],t,k;
  for(i=0;i<80;i++){
    if(i<20){t=(b&d)|((~b)&e);k=0x5A827999u;}
    else if(i<40){t=b^d^e;k=0x6ED9EBA1u;}
    else if(i<60){t=(b&d)|(b&e)|(d&e);k=0x8F1BBCDCu;}
    else{t=b^d^e;k=0xCA62C1D6u;}
    uint32_t tmp=_s1rol(a,5)+t+f+k+w[i];f=e;e=d;d=_s1rol(b,30);b=a;a=tmp;
  }
  c->h[0]+=a;c->h[1]+=b;c->h[2]+=d;c->h[3]+=e;c->h[4]+=f;
}
static inline int SHA1_Init(SHA_CTX*c){c->h[0]=0x67452301u;c->h[1]=0xEFCDAB89u;c->h[2]=0x98BADCFEu;c->h[3]=0x10325476u;c->h[4]=0xC3D2E1F0u;c->len=0;c->blen=0;return 1;}
static inline int SHA1_Update(SHA_CTX*c,const void*data,size_t n){
  const unsigned char*p=(const unsigned char*)data;c->len+=n;
  while(n){uint32_t need=64-c->blen,take=(n<need)?(uint32_t)n:need;memcpy(c->buf+c->blen,p,take);c->blen+=take;p+=take;n-=take;if(c->blen==64){_s1blk(c,c->buf);c->blen=0;}}
  return 1;
}
static inline int SHA1_Final(unsigned char*md,SHA_CTX*c){
  uint64_t bits=c->len*8;unsigned char pad=0x80,z=0;SHA1_Update(c,&pad,1);
  while(c->blen!=56)SHA1_Update(c,&z,1);
  unsigned char lb[8];for(int i=0;i<8;i++)lb[i]=(unsigned char)(bits>>(56-i*8));SHA1_Update(c,lb,8);
  for(int i=0;i<5;i++){md[i*4]=(unsigned char)(c->h[i]>>24);md[i*4+1]=(unsigned char)(c->h[i]>>16);md[i*4+2]=(unsigned char)(c->h[i]>>8);md[i*4+3]=(unsigned char)c->h[i];}
  return 1;
}
