# Bastion for testing SSH tunnels: password and key auth, TCP forwarding on.
FROM alpine:3.20
RUN apk add --no-cache openssh \
 && ssh-keygen -A \
 && adduser -D tablory && echo 'tablory:tablory' | chpasswd \
 && sed -i 's/^#\?AllowTcpForwarding.*/AllowTcpForwarding yes/; s/^#\?PasswordAuthentication.*/PasswordAuthentication yes/' /etc/ssh/sshd_config \
 && mkdir -p /home/tablory/.ssh
COPY id_ed25519.pub /home/tablory/.ssh/authorized_keys
RUN chown -R tablory /home/tablory/.ssh && chmod 600 /home/tablory/.ssh/authorized_keys
EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
