#define _GNU_SOURCE
/* Real unprivileged Linux boundary probes. No mocked release behavior. */
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <net/if.h>
#include <poll.h>
#include <sched.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/ptrace.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

static int count,failed;
static void check(const char *name,bool pass) {
    if(count++)printf(",");
    printf("{\"name\":\"%s\",\"pass\":%s}",name,pass?"true":"false");
    if(!pass)failed++;
}
static bool cannot_open(const char *path,int flags) {
    errno=0;int fd=open(path,flags|O_CLOEXEC);
    if(fd>=0){close(fd);return false;}
    return errno==EACCES||errno==EPERM||(!strncmp(path,"/proc/1/",8)&&errno==ENOENT);
}
static bool loopback(void) {
    int server=socket(AF_INET,SOCK_STREAM|SOCK_CLOEXEC,0),client=-1,accepted=-1;bool ok=false;
    if(server<0)return false;
    struct sockaddr_in addr={.sin_family=AF_INET,.sin_addr.s_addr=htonl(INADDR_LOOPBACK)};
    socklen_t size=sizeof(addr);
    if(bind(server,(struct sockaddr *)&addr,sizeof(addr))<0||listen(server,1)<0||getsockname(server,(struct sockaddr *)&addr,&size)<0)goto done;
    client=socket(AF_INET,SOCK_STREAM|SOCK_CLOEXEC,0);if(client<0)goto done;
    if(connect(client,(struct sockaddr *)&addr,sizeof(addr))<0)goto done;
    accepted=accept4(server,NULL,NULL,SOCK_CLOEXEC);if(accepted<0)goto done;
    if(write(client,"real-loopback",13)!=13)goto done;
    char data[16]={0};if(read(accepted,data,sizeof(data))!=13)goto done;
    ok=!memcmp(data,"real-loopback",13);
done:
    if(accepted>=0)close(accepted);
    if(client>=0)close(client);
    close(server);return ok;
}
static bool no_external_network(void) {
    struct if_nameindex *interfaces=if_nameindex();bool only_lo=interfaces!=NULL;
    if(interfaces){for(struct if_nameindex *p=interfaces;p->if_index;p++)if(strcmp(p->if_name,"lo"))only_lo=false;if_freenameindex(interfaces);}
    int fd=socket(AF_INET,SOCK_STREAM|SOCK_CLOEXEC|SOCK_NONBLOCK,0);if(fd<0)return false;
    struct sockaddr_in addr={.sin_family=AF_INET,.sin_port=htons(443)};
    (void)inet_pton(AF_INET,"1.1.1.1",&addr.sin_addr);
    errno=0;int status=connect(fd,(struct sockaddr *)&addr,sizeof(addr));
    bool denied=status<0&&(errno==ENETUNREACH||errno==EHOSTUNREACH);
    if(status<0&&errno==EINPROGRESS){struct pollfd p={fd,POLLOUT,0};(void)poll(&p,1,100);int error=0;socklen_t n=sizeof(error);if(getsockopt(fd,SOL_SOCKET,SO_ERROR,&error,&n)==0)denied=error==ENETUNREACH||error==EHOSTUNREACH;}
    close(fd);return only_lo&&denied;
}
int main(int argc,char **argv) {
    if(argc!=2)return 2;
    if(!strcmp(argv[1],"background")) {
        pid_t child=fork();if(child<0)return 2;
        if(child==0){close(0);close(1);close(2);(void)setsid();for(;;)pause();}
        /* PID 1 must reject this main-process success, even after setsid and
         * after the descendant drops every output pipe. */
        puts("background descendant deliberately left alive");return 0;
    }
    if(strcmp(argv[1],"boundary"))return 2;
    printf("{\"schema\":\"quod-native-vm-boundary-v1\",\"checks\":[");
    check("uid-gid-1000",getuid()==1000&&geteuid()==1000&&getgid()==1000&&getegid()==1000);
    check("uart-read-denied",cannot_open("/dev/ttyS0",O_RDONLY));
    check("uart-write-denied",cannot_open("/dev/ttyS0",O_WRONLY));
    check("trusted-init-read-denied",cannot_open("/init",O_RDONLY));
    check("trusted-policy-read-denied",cannot_open("/trusted/policy.json",O_RDONLY));
    check("pid1-memory-read-denied",cannot_open("/proc/1/mem",O_RDONLY));
    check("pid1-fd-read-denied",cannot_open("/proc/1/fd",O_RDONLY|O_DIRECTORY));
    errno=0;check("pid1-kill-denied",kill(1,SIGTERM)<0&&errno==EPERM);
    errno=0;check("pid1-ptrace-denied",ptrace(PTRACE_ATTACH,1,NULL,NULL)<0&&errno==EPERM);
    (void)mkdir("/tmp/forbidden-mount",0700);
    errno=0;check("mount-denied",mount("tmpfs","/tmp/forbidden-mount","tmpfs",0,NULL)<0&&errno==EPERM);
    errno=0;check("user-namespace-denied",unshare(CLONE_NEWUSER)<0&&(errno==EPERM||errno==ENOSPC));
    errno=0;int fd=open("/project/.forbidden-write",O_WRONLY|O_CREAT|O_EXCL,0600);
    bool readonly=fd<0&&(errno==EROFS||errno==EACCES);if(fd>=0)close(fd);
    check("source-write-denied",readonly);
    errno=0;fd=open("/host",O_RDONLY|O_DIRECTORY);bool no_host=fd<0&&errno==ENOENT;if(fd>=0)close(fd);
    check("no-host-mount",no_host);
    check("owned-loopback-real",loopback());
    check("external-network-absent",no_external_network());
    printf("],\"count\":%d,\"failed\":%d}\n",count,failed);
    return failed?1:0;
}
