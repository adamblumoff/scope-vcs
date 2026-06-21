For railway deployment stuff, if you cannot do it via the cli, do not add more configs, just tell me how to do it via the dashboard. 

This product is still in pre-alpha. Every change should be destructive, no legacy or backwards compatible bullshit, this application has no users or releases, treat it as such. If you add backwards compatibility, I will go apeshit on your dumbass.

Please don't use cards for ui, only use them if absolutely necessary.

No file should be over 1000 lines of code, at that point do an audit of the file and modularize.

We have PR environments enabled on Railway, so before you merge, manual qa with that environment accordingly and make sure the changes are solid.