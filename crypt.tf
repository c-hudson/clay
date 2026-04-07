;
;
; encrypt.tf
;    This is an implimentation of some really simple encryption.
;    Its probably slightly more effective then say rot13. Don't
;    trust this code to deter dedicated people. Trust this code
;    to baffle newbies.
;
; Useage:
;    /e <text>                 Encrypts <text> using the password set by
;                              the /passwd command.
;    /passwd <text>            Set the password to <text>.
;

/def random = /echo -- %R

/def passwd = \
   /let i=0%;\
   /let eol=$[strlen({*})]%;\
   /while (i < eol) \
      /let char=$[ascii(substr({*},i,1))]%;\
      /if (char >= 32) \
         /if (char <=  126) \
            /let tmppwd=%tmppwd$[char(char)]%;\
         /endif%;\
      /endif%;\
      /test ++i%;\
   /done%;\
   /def crypt_pwd=%tmppwd%;\
   
/def encrypt = \
   /let i=0%;\
    /while (i < strlen({*})) \
      /let char=$[mod(ascii(substr({*},i,1)) + \
         ascii(substr(${crypt_pwd},mod(i,strlen(${crypt_pwd})),1)) - \
         64,95)+32]%;\
      /let printable=x$(/makeprintable %{i} %{char})x%;\
      /let result=%result$[substr(printable,1,strlen(printable)-2)]%;\
      /test ++i%;\
   /done%;\
   /echo -- %result%;\

/def decrypt = \
   /let i=1%;\
   /let j=0%;\
   /while (i < (strlen({-1}) - 1)) \
      /let char=$[ascii(substr({-1},i,1))]%;\
      /if ({1} & char == 92) \
         /let char=$[ascii(substr({-1},++i,1))]%;\
      /elseif ({1} & (substr({-1},i,2)) =/ "%b") \
         /let char=32%;\
         /test ++i%;\
      /endif%;\
      /let code=$[substr(code,0,strlen(code)-1)]$[char(mod({char} - \
         ascii(substr(${crypt_pwd},j,1)) + 190,95) + 32)]a%;\
      /let j=$[mod(++j,strlen(${crypt_pwd}))]%;\
      /test ++i%;\
   /done%;\
   /echo -- $[substr(code,0,strlen(code)-1)]

/def makeprintable = \
   /if ({-1} == 32) \
      /echo -- \%b%;\
   /elseif ({1} == 0) \
      /echo -- $[char({-1})]%;\
   /elseif ({-1}==92 | {-1}==91 | {-1}==93 | {-1}==123 | {-1}==125 | {-1}==37) \
      /echo -- \\$[char({-1})]%;\
   /else \
      /echo -- $[char({-1})]%;\
   /endif

/def e = \
   /echo -- say \\$(/encrypt %*3.14)%;\
   say \\$(/encrypt %*3.14)

/def p = \
   +pub \\$(/encrypt %*3.14)

/def -p5000 -mregexp -t' (say|says|says,|say,) "(.*)"$$' \
      listen_mush = \
   /if (substr({P2},0,1) =~ "\\") \
   	/let dcrypt=$(/decrypt 1 x%P2x)%;\
   /else \
        /let dcrypt=$(/decrypt 0 x%P2x)%;\
   /endif%;\
   /if (dcrypt =/ "*3.14") \
      /if (dcrypt =/ "\:*") \
         /echo -w${world_name} -ag -- %*%;\
         /substitute -aCred -- %% * %PL $[substr(dcrypt,strstr(dcrypt,":")+1,\
            strlen(dcrypt)-5)]%;\
      /else \
         /echo -w${world_name} -ag -- %*%;\
         /substitute -aCred -- %% %PL %P1 \
            "$[substr(dcrypt,0,strlen(dcrypt)-4)]"%;\
      /endif%;\
   /endif

;/passwd welcometoencryptionpartyongarth
/passwd Fredrik
; /passwd test
