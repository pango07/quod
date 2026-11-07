#define _GNU_SOURCE
/* Trusted PID 1. No child can access its policy, file descriptors or UART.
 * This binary issues investigation evidence only; it never admits a receipt. */
#include <arpa/inet.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <grp.h>
#include <linux/random.h>
#include <limits.h>
#include <net/if.h>
#include <poll.h>
#include <signal.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mount.h>
#include <sys/personality.h>
#include <sys/prctl.h>
#include <sys/random.h>
#include <sys/reboot.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/sysmacros.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

#define CAPTURE_LIMIT (16U * 1024U * 1024U)
#define OUTPUT_LIMIT (48U * 1024U * 1024U)
#define BODY_LIMIT (96U * 1024U * 1024U)
#define EPOCH 1700000000

typedef struct { unsigned char *data; size_t len, capacity, limit; } Buffer;
static char failure[512];
static bool entropy_ready, loopback_ready, source_readonly, child_exited, capture_complete;
static unsigned char entropy_probe[32];
static int command_status = -1, leftovers_detected, killed_processes;
static size_t output_bytes, output_entries;
static const char *command;

static void fail(const char *fmt, ...) {
    if (failure[0]) return;
    va_list ap; va_start(ap, fmt); vsnprintf(failure, sizeof(failure), fmt, ap); va_end(ap);
}
static bool append(Buffer *b, const void *data, size_t n) {
    if (n > b->limit - b->len) { fail("bounded output exceeded"); return false; }
    if (b->len + n + 1 > b->capacity) {
        size_t next = b->capacity ? b->capacity * 2 : 4096;
        if (next < b->len + n + 1) next = b->len + n + 1;
        unsigned char *p = realloc(b->data, next);
        if (!p) { fail("allocation failed"); return false; }
        b->data = p; b->capacity = next;
    }
    if (n) memcpy(b->data + b->len, data, n);
    b->len += n; b->data[b->len] = 0; return true;
}
static void text(Buffer *b, const char *s) { (void)append(b, s, strlen(s)); }
static void format(Buffer *b, const char *fmt, ...) {
    char data[1024]; va_list ap; va_start(ap, fmt);
    int n = vsnprintf(data, sizeof(data), fmt, ap); va_end(ap);
    if (n < 0 || (size_t)n >= sizeof(data)) { fail("format overflow"); return; }
    (void)append(b, data, (size_t)n);
}
static void quoted(Buffer *b, const char *s) {
    text(b, "\"");
    for (const unsigned char *p = (const unsigned char *)s; *p; p++) {
        if (*p == '"' || *p == '\\') { char x[2] = {'\\', (char)*p}; (void)append(b, x, 2); }
        else if (*p < 32 || *p >= 127) format(b, "\\u%04x", (unsigned)*p);
        else (void)append(b, p, 1);
    }
    text(b, "\"");
}
static void base64(Buffer *b, const unsigned char *p, size_t n) {
    static const char a[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    text(b, "\"");
    for (size_t i = 0; i < n; i += 3) {
        unsigned x = (unsigned)p[i] << 16;
        if (i + 1 < n) x |= (unsigned)p[i + 1] << 8;
        if (i + 2 < n) x |= p[i + 2];
        char out[4] = {a[(x >> 18) & 63], a[(x >> 12) & 63],
                       i + 1 < n ? a[(x >> 6) & 63] : '=', i + 2 < n ? a[x & 63] : '='};
        if (!append(b, out, 4)) break;
    }
    text(b, "\"");
}

/* SHA-256 (FIPS 180-4). No guest executable is invoked to authenticate output. */
typedef struct { uint32_t h[8]; uint64_t bytes; unsigned char block[64]; size_t used; } Sha;
static uint32_t rotr(uint32_t x, unsigned n) { return (x >> n) | (x << (32 - n)); }
static void sha_block(Sha *s, const unsigned char *p) {
    static const uint32_t k[64] = {
        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2};
    uint32_t w[64];
    for (unsigned i = 0; i < 16; i++) w[i] = ((uint32_t)p[i*4] << 24) | ((uint32_t)p[i*4+1] << 16) | ((uint32_t)p[i*4+2] << 8) | p[i*4+3];
    for (unsigned i = 16; i < 64; i++) {
        uint32_t x = w[i-15], y = w[i-2];
        w[i] = w[i-16] + (rotr(x,7)^rotr(x,18)^(x>>3)) + w[i-7] + (rotr(y,17)^rotr(y,19)^(y>>10));
    }
    uint32_t a=s->h[0],b=s->h[1],c=s->h[2],d=s->h[3],e=s->h[4],f=s->h[5],g=s->h[6],h=s->h[7];
    for (unsigned i=0; i<64; i++) {
        uint32_t t1=h+(rotr(e,6)^rotr(e,11)^rotr(e,25))+((e&f)^(~e&g))+k[i]+w[i];
        uint32_t t2=(rotr(a,2)^rotr(a,13)^rotr(a,22))+((a&b)^(a&c)^(b&c));
        h=g;g=f;f=e;e=d+t1;d=c;c=b;b=a;a=t1+t2;
    }
    s->h[0]+=a;s->h[1]+=b;s->h[2]+=c;s->h[3]+=d;s->h[4]+=e;s->h[5]+=f;s->h[6]+=g;s->h[7]+=h;
}
static void sha_init(Sha *s) {
    *s = (Sha){.h={0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19}};
}
static void sha_add(Sha *s, const unsigned char *p, size_t n) {
    s->bytes += n;
    while (n) {
        size_t take=64-s->used; if (take>n) take=n;
        memcpy(s->block+s->used,p,take);s->used+=take;p+=take;n-=take;
        if (s->used==64) {sha_block(s,s->block);s->used=0;}
    }
}
static void sha_end(Sha *s, char hex[65]) {
    uint64_t bits=s->bytes*8; unsigned char pad[128]={0x80};
    size_t n=s->used<56?56-s->used:120-s->used;
    sha_add(s,pad,n);
    for(unsigned i=0;i<8;i++) pad[i]=(unsigned char)(bits>>(56-i*8));
    sha_add(s,pad,8);
    for(unsigned i=0;i<8;i++) (void)sprintf(hex+i*8,"%08x",s->h[i]);
    hex[64]=0;
}
static void hash(const unsigned char *p, size_t n, char out[65]) { Sha s;sha_init(&s);sha_add(&s,p,n);sha_end(&s,out); }
static bool write_all(int fd, const void *data, size_t n) {
    const unsigned char *p=data;
    while(n) {ssize_t k=write(fd,p,n);if(k<0&&errno==EINTR)continue;if(k<=0)return false;p+=k;n-=(size_t)k;}
    return true;
}
static bool write_file(const char *path, const char *value) {
    int fd=open(path,O_WRONLY|O_CLOEXEC);if(fd<0)return false;
    bool ok=write_all(fd,value,strlen(value));close(fd);return ok;
}
static void make_device(const char *path, unsigned major_no, unsigned minor_no, mode_t mode) {
    if(mknod(path,S_IFCHR|mode,makedev(major_no,minor_no))<0 && errno!=EEXIST) fail("device creation %s: %s",path,strerror(errno));
    if(chmod(path,mode)<0) fail("device mode %s",path);
}
static void setup(void) {
    if(getpid()!=1 || getuid()!=0) {fail("must run as root PID 1");return;}
    if(prctl(PR_SET_DUMPABLE,0)<0 || prctl(PR_SET_CHILD_SUBREAPER,1)<0) fail("supervisor prctl");
    if(sethostname("quod-native-vm",14)<0) fail("hostname");
    if(mount("proc","/proc","proc",MS_NOSUID|MS_NODEV|MS_NOEXEC,"hidepid=2")<0) fail("private proc mount: %s",strerror(errno));
    if(mount("tmpfs","/dev","tmpfs",MS_NOSUID,"mode=0755,size=8m")<0) fail("private dev mount");
    make_device("/dev/null",1,3,0666);make_device("/dev/zero",1,5,0666);
    make_device("/dev/random",1,8,0666);make_device("/dev/urandom",1,9,0666);
    make_device("/dev/ttyS0",4,64,0600);
    if(mkdir("/dev/shm",01777)<0 && errno!=EEXIST) fail("shm directory");
    if(mount("tmpfs","/dev/shm","tmpfs",MS_NOSUID|MS_NODEV,"mode=1777,size=256m")<0) fail("shm mount");
    if(mount("tmpfs","/tmp","tmpfs",MS_NOSUID|MS_NODEV,"mode=1777,size=1024m")<0) fail("scratch mount");
    if(mount("tmpfs","/run","tmpfs",MS_NOSUID|MS_NODEV,"mode=0755,size=8m")<0) fail("run mount");
    if(mkdir("/tmp/home",0700)<0 || chown("/tmp/home",1000,1000)<0) fail("isolated home");
    if(!write_file("/proc/sys/kernel/randomize_va_space","0\n")) fail("ASLR policy");
    if(!write_file("/proc/sys/user/max_user_namespaces","0\n")) fail("user namespace policy");
    if(personality(ADDR_NO_RANDOMIZE)<0) fail("fixed address policy");
    struct { int entropy_count; int buf_size; unsigned char buf[64]; } seed={.entropy_count=512,.buf_size=64};
    for(unsigned i=0;i<sizeof(seed.buf);i++)seed.buf[i]=(unsigned char)(0x51U+i*73U);
    int random_fd=open("/dev/random",O_RDWR|O_CLOEXEC);
    if(random_fd<0 || ioctl(random_fd,RNDADDENTROPY,&seed)<0) fail("deterministic root entropy seed");
    if(random_fd>=0)close(random_fd);
    if(getrandom(entropy_probe,sizeof(entropy_probe),GRND_NONBLOCK)!=(ssize_t)sizeof(entropy_probe)) fail("kernel PRNG was not ready before child");
    else entropy_ready=true;
    int sock=socket(AF_INET,SOCK_DGRAM|SOCK_CLOEXEC,0);
    struct ifreq request;memset(&request,0,sizeof(request));strcpy(request.ifr_name,"lo");
    if(sock<0 || ioctl(sock,SIOCGIFFLAGS,&request)<0) fail("loopback query");
    else {request.ifr_flags|=IFF_UP|IFF_RUNNING;if(ioctl(sock,SIOCSIFFLAGS,&request)<0)fail("loopback enable");else loopback_ready=true;}
    if(sock>=0)close(sock);
    if(mount("/project","/project",NULL,MS_BIND|MS_REC,NULL)<0 ||
       mount(NULL,"/project",NULL,MS_REMOUNT|MS_BIND|MS_RDONLY|MS_NOSUID|MS_NODEV,NULL)<0) fail("read-only project boundary");
    else source_readonly=true;
    if(mount("tmpfs","/project/dist","tmpfs",MS_NOSUID|MS_NODEV,"mode=0755,uid=1000,gid=1000,size=256m")<0) fail("dist output mount");
}
static void read_command(void) {
    char data[8192];int fd=open("/proc/cmdline",O_RDONLY|O_CLOEXEC);ssize_t n=fd<0?-1:read(fd,data,sizeof(data)-1);if(fd>=0)close(fd);
    if(n<=0){fail("missing fixed command");return;}data[n]=0;
    char *p=strstr(data,"quod.command=");if(!p){fail("missing command selector");return;}p+=13;
    if(strncmp(p,"boundary",8)==0 && (p[8]==' '||p[8]=='\n'||p[8]==0))command="boundary";
    else if(strncmp(p,"background",10)==0 && (p[10]==' '||p[10]=='\n'||p[10]==0))command="background";
    else if(strncmp(p,"build",5)==0 && (p[5]==' '||p[5]=='\n'||p[5]==0))command="build";
    else if(strncmp(p,"release",7)==0 && (p[7]==' '||p[7]=='\n'||p[7]==0))command="release";
    else fail("unknown command selector");
}
static int owned_processes(bool kill_them) {
    DIR *dir=opendir("/proc");if(!dir){fail("cannot inspect process completion");return -1;}
    int count=0;struct dirent *ent;
    while((ent=readdir(dir))) {
        char *end;long pid=strtol(ent->d_name,&end,10);if(*end||pid<=1)continue;
        char path[64];(void)snprintf(path,sizeof(path),"/proc/%ld/status",pid);
        FILE *f=fopen(path,"r");if(!f)continue;char line[512];unsigned uid=UINT_MAX;
        while(fgets(line,sizeof(line),f))if(sscanf(line,"Uid:\t%u",&uid)==1)break;
        fclose(f);
        if(uid==1000){count++;if(kill_them && kill((pid_t)pid,SIGKILL)==0)killed_processes++;}
    }
    closedir(dir);return count;
}
static void reap_all(pid_t main_pid) {
    int status;pid_t pid;
    while((pid=waitpid(-1,&status,WNOHANG))>0)if(pid==main_pid){child_exited=true;command_status=status;}
}
static void execute(Buffer *out,Buffer *err) {
    int output[2],errors[2];
    if(pipe2(output,O_CLOEXEC)<0||pipe2(errors,O_CLOEXEC)<0){fail("capture pipes");return;}
    pid_t pid=fork();
    if(pid<0){fail("fork command");return;}
    if(pid==0) {
        int nullfd=open("/dev/null",O_RDONLY);if(nullfd<0)_exit(120);
        if(dup2(nullfd,0)<0||dup2(output[1],1)<0||dup2(errors[1],2)<0)_exit(121);
        if(syscall(SYS_close_range,3U,~0U,0U)<0)for(int fd=3;fd<65536;fd++)close(fd);
        if(setgroups(0,NULL)<0||setresgid(1000,1000,1000)<0||setresuid(1000,1000,1000)<0)_exit(122);
        if(prctl(PR_SET_NO_NEW_PRIVS,1,0,0,0)<0||prctl(PR_SET_DUMPABLE,0)<0)_exit(123);
        struct rlimit core={0,0};if(setrlimit(RLIMIT_CORE,&core)<0)_exit(124);
        umask(0022);if(chdir("/project")<0)_exit(125);
        char *const env[]={"PATH=/tools/node/bin:/usr/bin:/bin:/usr/lib/git-core","HOME=/tmp/home","TMPDIR=/tmp",
            "TZ=UTC","LANG=C.UTF-8","LC_ALL=C.UTF-8","USER=quod","LOGNAME=quod","SHELL=/bin/sh",
            "NODE_ENV=test","GIT_CONFIG_NOSYSTEM=1","GIT_TERMINAL_PROMPT=0","npm_config_offline=true",
            "npm_config_audit=false","npm_config_fund=false","npm_config_update_notifier=false",NULL};
        if(!strcmp(command,"boundary")||!strcmp(command,"background")) {
            char *const argv[]={"/bin/quod-negative",(char *)command,NULL};execve(argv[0],argv,env);
        } else {
            char *const argv[]={"/tools/node/bin/node","/tools/node/lib/node_modules/npm/bin/npm-cli.js","run",
                !strcmp(command,"build")?"build":"test:release",NULL};execve(argv[0],argv,env);
        }
        _exit(126);
    }
    close(output[1]);close(errors[1]);
    (void)fcntl(output[0],F_SETFL,O_NONBLOCK);(void)fcntl(errors[0],F_SETFL,O_NONBLOCK);
    struct pollfd fds[2]={{output[0],POLLIN,0},{errors[0],POLLIN,0}};
    bool eof[2]={false,false},cleaned=false;
    struct timespec start,now;(void)clock_gettime(CLOCK_MONOTONIC,&start);
    for(;;) {
        (void)poll(fds,2,20);
        for(unsigned i=0;i<2;i++) {
            if(eof[i])continue;
            unsigned char chunk[8192];ssize_t n;
            while((n=read(fds[i].fd,chunk,sizeof(chunk)))>0) (void)append(i?err:out,chunk,(size_t)n);
            if(n==0){eof[i]=true;close(fds[i].fd);fds[i].fd=-1;}
            else if(errno!=EAGAIN && errno!=EWOULDBLOCK && errno!=EINTR){fail("capture read");eof[i]=true;close(fds[i].fd);fds[i].fd=-1;}
        }
        reap_all(pid);
        if(child_exited&&!cleaned) {
            int remaining=owned_processes(false);
            if(remaining>0){leftovers_detected=remaining;fail("unprivileged processes remained after command exit");(void)owned_processes(true);}
            cleaned=true;
        }
        (void)clock_gettime(CLOCK_MONOTONIC,&now);
        if(now.tv_sec-start.tv_sec>300){fail("guest virtual deadline exceeded");(void)owned_processes(true);}
        if(failure[0]&&!child_exited)(void)owned_processes(true);
        if(child_exited&&eof[0]&&eof[1])break;
    }
    reap_all(pid);
    /* Kill/reap is bounded by the host watchdog too. An unkillable process
     * never produces a successful envelope. */
    for(unsigned i=0;i<1000;i++) {
        reap_all(pid);int n=owned_processes(false);
        if(n==0){capture_complete=true;break;}
        (void)owned_processes(true);struct timespec pause={0,1000000};(void)nanosleep(&pause,NULL);
    }
    if(!capture_complete)fail("process quiescence not established");
}
static void dist_tree(Buffer *b,const char *relative) {
    char path[PATH_MAX];int n=snprintf(path,sizeof(path),"/project/dist%s%s",*relative?"/":"",relative);
    if(n<0||(size_t)n>=sizeof(path)){fail("output path bound");return;}
    struct stat st;if(lstat(path,&st)<0){fail("output lstat");return;}
    {
        if(++output_entries>50000){fail("output entry bound");return;}
        if(output_entries>1)text(b,",");
        text(b,"{\"pathBase64\":");base64(b,(const unsigned char *)relative,strlen(relative));
        format(b,",\"mode\":%u,\"uid\":%u,\"gid\":%u,\"inode\":%llu,\"device\":%llu,\"links\":%llu,\"mtimeSeconds\":%lld,\"mtimeNanoseconds\":%ld,\"atimeSeconds\":%lld,\"atimeNanoseconds\":%ld,\"ctimeSeconds\":%lld,\"ctimeNanoseconds\":%ld",
            (unsigned)(st.st_mode&07777),(unsigned)st.st_uid,(unsigned)st.st_gid,
            (unsigned long long)st.st_ino,(unsigned long long)st.st_dev,(unsigned long long)st.st_nlink,
            (long long)st.st_mtim.tv_sec,st.st_mtim.tv_nsec,(long long)st.st_atim.tv_sec,st.st_atim.tv_nsec,
            (long long)st.st_ctim.tv_sec,st.st_ctim.tv_nsec);
        if(S_ISREG(st.st_mode)) {
            if(st.st_size<0||(uint64_t)st.st_size>OUTPUT_LIMIT-output_bytes){fail("output byte bound");return;}
            size_t size=(size_t)st.st_size;unsigned char *data=malloc(size?size:1);
            if(!data){fail("output allocation");return;}
            int fd=open(path,O_RDONLY|O_NOFOLLOW|O_CLOEXEC);size_t got=0;
            if(fd<0){free(data);fail("output open");return;}
            while(got<size){ssize_t k=read(fd,data+got,size-got);if(k<0&&errno==EINTR)continue;if(k<=0)break;got+=(size_t)k;}
            close(fd);if(got!=size){free(data);fail("output read");return;}output_bytes+=size;
            char sha[65];hash(data,size,sha);text(b,",\"type\":\"file\",\"sha256\":");quoted(b,sha);
            format(b,",\"bytes\":%zu,\"dataBase64\":",size);base64(b,data,size);free(data);
        } else if(S_ISLNK(st.st_mode)) {
            unsigned char target[PATH_MAX];ssize_t k=readlink(path,(char *)target,sizeof(target));
            if(k<0||(size_t)k==sizeof(target)){fail("output symlink bound");return;}
            text(b,",\"type\":\"symlink\",\"targetBase64\":");base64(b,target,(size_t)k);
        } else if(S_ISDIR(st.st_mode))text(b,",\"type\":\"directory\"");
        else {fail("special output file");return;}
        text(b,"}");
    }
    if(S_ISDIR(st.st_mode)) {
        struct dirent **entries=NULL;int count=scandir(path,&entries,NULL,alphasort);
        if(count<0){fail("output directory");return;}
        for(int i=0;i<count;i++) {
            const char *name=entries[i]->d_name;
            if(strcmp(name,".")&&strcmp(name,"..")) {
                char child[PATH_MAX];int k=snprintf(child,sizeof(child),"%s%s%s",relative,*relative?"/":"",name);
                if(k<0||(size_t)k>=sizeof(child))fail("output path length");else if(!failure[0])dist_tree(b,child);
            }
            free(entries[i]);
        }
        free(entries);
    }
}
static void emit(Buffer *out,Buffer *err,Buffer *dist) {
    Buffer body={.limit=BODY_LIMIT};
    bool complete=!failure[0]&&child_exited&&capture_complete&&leftovers_detected==0;
    int exit_code=child_exited&&WIFEXITED(command_status)?WEXITSTATUS(command_status):-1;
    bool ok=complete&&exit_code==0;
    text(&body,"{\"schema\":\"quod-native-vm-response-v1-investigation\",\"contract\":\"diskless-canonical-linux-native-v1-investigation\",\"command\":");
    quoted(&body,command?command:"unknown");
    format(&body,",\"ok\":%s,\"complete\":%s,\"exitCode\":%d,\"signal\":%d,\"leftoversDetected\":%d,\"killedProcesses\":%d,\"failure\":",
        ok?"true":"false",complete?"true":"false",exit_code,child_exited&&WIFSIGNALED(command_status)?WTERMSIG(command_status):0,leftovers_detected,killed_processes);
    quoted(&body,failure);
    format(&body,",\"proof\":{\"entropySeededBeforeChild\":%s,\"loopbackEnabled\":%s,\"sourceReadOnly\":%s,\"captureComplete\":%s,\"remainingProcesses\":%d,\"entropyProbeBase64\":",
        entropy_ready?"true":"false",loopback_ready?"true":"false",source_readonly?"true":"false",capture_complete?"true":"false",owned_processes(false));
    base64(&body,entropy_probe,sizeof(entropy_probe));
    text(&body,"},\"effects\":{\"stdoutBase64\":");base64(&body,out->data,out->len);
    text(&body,",\"stderrBase64\":");base64(&body,err->data,err->len);
    format(&body,",\"distEntries\":%zu,\"distBytes\":%zu,\"dist\":[",output_entries,output_bytes);
    (void)append(&body,dist->data,dist->len);text(&body,"]},\"receiptAdmission\":false}");
    /* An allocation/format failure while serializing must not retain an earlier
     * ok:true. Emit no envelope rather than an incomplete success document. */
    if(ok&&failure[0]){free(body.data);return;}
    char sha[65];hash(body.data,body.len,sha);
    int serial=open("/dev/ttyS0",O_WRONLY|O_NOCTTY|O_CLOEXEC);
    if(serial<0){free(body.data);return;}
    struct termios term;if(tcgetattr(serial,&term)==0){cfmakeraw(&term);(void)tcsetattr(serial,TCSANOW,&term);}
    char header[160];int n=snprintf(header,sizeof(header),"\nQUOD_NATIVE_VM_RESULT_V1 %zu %s\n",body.len,sha);
    if(n>0&&(size_t)n<sizeof(header)&&write_all(serial,header,(size_t)n)&&write_all(serial,body.data,body.len))
        (void)write_all(serial,"\nQUOD_NATIVE_VM_END\n",20);
    (void)tcdrain(serial);close(serial);free(body.data);
}
int main(int argc,char **argv) {
    if(argc==2&&!strcmp(argv[1],"--self-test")) {
        char actual[65];hash((const unsigned char *)"abc",3,actual);
        return strcmp(actual,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
    Buffer out={.limit=CAPTURE_LIMIT},err={.limit=CAPTURE_LIMIT},dist={.limit=BODY_LIMIT};
    setup();read_command();if(!failure[0])execute(&out,&err);
    if(!failure[0])dist_tree(&dist,"");
    emit(&out,&err,&dist);
    free(out.data);free(err.data);free(dist.data);sync();
    (void)reboot(RB_POWER_OFF);
    for(;;)pause();
}
